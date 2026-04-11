use crate::arch::Arch;
use crate::arch::ArchTrait;
use crate::fs::vfs::INode;
use crate::fs::vfs::{DirectoryTrait, FileTrait, Filesystem};
use crate::physical_memory::HHDM_OFFSET;
use crate::sync::IntMutex;
use crate::sync::MutexLike;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use spin::Once;

pub struct RAMFilesystem {
    reference: Once<Arc<RAMFilesystem>>,
    files: IntMutex<Vec<Arc<dyn INode>>>,
}

impl DirectoryTrait for IntMutex<Vec<(String, usize)>> {
    fn lookup(&self, target: &str) -> Result<usize, &'static str> {
        for (name, inumber) in &*(self.lock()) {
            if *name == *target {
                return Ok(*inumber);
            }
        }
        Err("could not find file")
    }
    fn add_entry(&self, target: &str, inumber: usize) -> Result<(), &'static str> {
        self.lock().push((target.into(), inumber));
        Ok(())
    }
}

impl FileTrait for IntMutex<Vec<u8>> {
    fn read_page(&self, physical_address: *mut u8, offset: usize) -> Result<(), &'static str> {
        if !offset.is_multiple_of(Arch::PAGE_SIZE) {
            return Err("given offset is not multiple of page size");
        }
        let content = self.lock();
        if offset > content.len() {
            return Err("given offset is above file size");
        }
        let length = Arch::PAGE_SIZE.min(content.len() - offset);
        let hhdm = *HHDM_OFFSET.get().unwrap();
        for i in 0..length {
            let adjusted = (physical_address as usize) + hhdm + i;
            let adjusted = adjusted as *mut u8;
            unsafe { *adjusted = content[offset + i] }
        }
        Ok(())
    }

    fn write_page(&self, physical_address: *const u8, offset: usize) -> Result<(), &'static str> {
        if !offset.is_multiple_of(Arch::PAGE_SIZE) {
            return Err("given offset is not multiple of page size");
        }
        let mut content = self.lock();
        while offset + Arch::PAGE_SIZE > content.len() {
            content.push(0);
        }
        let length = Arch::PAGE_SIZE.min(content.len() - offset);
        let hhdm = *HHDM_OFFSET.get().unwrap();
        for i in 0..length {
            let adjusted = (physical_address as usize) + hhdm + i;
            let adjusted = adjusted as *mut u8;
            content[offset + i] = unsafe { *adjusted }
        }
        Ok(())
    }
}

impl RAMFilesystem {
    pub fn new() -> Arc<Self> {
        let fs: Arc<RAMFilesystem> = Arc::new(Self {
            reference: Once::new(),
            files: IntMutex::new(vec![]),
        });
        fs.reference.call_once(|| fs.clone());
        fs
    }
}

impl Filesystem for RAMFilesystem {
    fn get_inode(&self, inumber: usize) -> Option<Arc<dyn INode>> {
        Some(self.files.lock()[inumber].clone())
    }
    fn get_root(&self) -> Arc<dyn INode> {
        self.get_inode(0).unwrap()
    }
}

#[cfg(test)]
mod test {
    use crate::arch::{Arch, ArchTrait};
    use crate::fs::ramfs::RAMFilesystem;
    use crate::fs::vfs::Filesystem;
    use crate::fs::vfs::{Directory, File};
    use crate::physical_memory;
    use crate::physical_memory::HHDM_OFFSET;
    use crate::sync::IntMutex;
    use alloc::string::String;
    use alloc::sync::Arc;
    use alloc::vec::Vec;

    fn init_test_ramfs(fs: Arc<RAMFilesystem>) {
        let empty = Arc::new(Directory(IntMutex::new(Vec::<(String, usize)>::new())));
        let Some(root) = fs.get_inode(fs.create_inode(empty)) else {
            panic!("created inode doesn't exist");
        };
        let _ = root.add_entry("small", 1);
        let _ = root.add_entry("big", 2);
        // add(RAMINodeContainer::Directory(vec![("small", 1), ("big", 2)]));

        let small = fs.create_inode(Arc::new(File(IntMutex::new(Vec::<u8>::new()))));
        let Some(small) = fs.get_inode(small) else {
            panic!("created inode doesn't exist")
        };
        let paddr = physical_memory::frame_alloc() as *mut u8;
        let hhdm = *HHDM_OFFSET.get().unwrap();
        let mut i = 0;
        for c in "cat".as_bytes().to_vec() {
            unsafe { *(paddr.wrapping_add(hhdm).wrapping_add(i)) = c };
            i += 1;
        }
        let _ = small.write_page(paddr, 0);

        let big = fs.create_inode(Arc::new(File(IntMutex::new(Vec::<u8>::new()))));
        let Some(big) = fs.get_inode(big) else {
            panic!("created inode doesn't exist")
        };
        for j in 0..4 {
            i = 0;
            for _ in 0..1024 {
                for c in "cats".as_bytes().to_vec() {
                    unsafe { *(paddr.wrapping_add(hhdm).wrapping_add(i)) = c };
                    i += 1;
                }
            }
            let _ = big.write_page(paddr, j * Arch::PAGE_SIZE);
        }
    }

    #[test_case]
    fn test_ramfs_small() {
        let fs = RAMFilesystem::new();
        init_test_ramfs(fs.clone());

        let root = fs.get_root();
        let small = fs.get_inode(root.lookup("small").unwrap()).unwrap();
        let paddr = physical_memory::frame_alloc();
        let hhdm = *HHDM_OFFSET.get().unwrap();

        small.read_page(paddr as *mut u8, 0).unwrap();
        unsafe {
            assert!(*((paddr + hhdm) as *const u8) == b'c');
            assert!(*((paddr + hhdm + 1) as *const u8) == b'a');
            assert!(*((paddr + hhdm + 2) as *const u8) == b't');
            *((paddr + hhdm) as *mut u8) = b'b';
        }

        small.write_page(paddr as *const u8, 0).unwrap();
        unsafe { *((paddr + hhdm) as *mut u8) = b'c' };

        small.read_page(paddr as *mut u8, 0).unwrap();
        unsafe {
            let b = *((paddr + hhdm) as *const u8);
            assert!(b == b'b');
        }

        physical_memory::frame_dealloc(paddr);
    }

    #[test_case]
    fn test_ramfs_big() {
        let fs = RAMFilesystem::new();
        init_test_ramfs(fs.clone());

        let root = fs.get_root();
        let big = fs.get_inode(root.lookup("big").unwrap()).unwrap();
        let paddr = physical_memory::frame_alloc();
        let hhdm = *HHDM_OFFSET.get().unwrap();

        // TODO change below variable names lol
        for j in 0..4 {
            big.read_page(paddr as *mut u8, j * Arch::PAGE_SIZE)
                .unwrap();
            for i in 0..1024 {
                unsafe {
                    let c = *((paddr + hhdm + 4 * i) as *const u8);
                    let a = *((paddr + hhdm + 4 * i + 1) as *const u8);
                    let t = *((paddr + hhdm + 4 * i + 2) as *const u8);
                    let s = *((paddr + hhdm + 4 * i + 3) as *const u8);
                    assert!(c == b'c');
                    assert!(a == b'a');
                    assert!(t == b't');
                    assert!(s == b's');
                }
            }
        }

        physical_memory::frame_dealloc(paddr);
    }
}
