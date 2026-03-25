use crate::arch::Arch;
use crate::arch::ArchTrait;
use crate::physical_memory::HHDM_OFFSET;
use crate::sync::IntMutex;
use crate::sync::MutexLike;
use crate::vfs::Filesystem;
use crate::vfs::INode;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

pub struct RAMFilesystem {
    files: IntMutex<Vec<Arc<RAMINode>>>,
}

enum RAMINodeContainer {
    Directory(Vec<(&'static str, usize)>),
    File(IntMutex<Vec<u8>>),
}

struct RAMINode {
    filesystem: Arc<RAMFilesystem>,
    container: RAMINodeContainer,
}

impl RAMFilesystem {
    // TODO: Not hardcode files into the constructor...
    pub fn new() -> Arc<Self> {
        let fs: Arc<RAMFilesystem> = Arc::new(Self {
            files: IntMutex::new(vec![]),
        });
        let add = |file| {
            let strong = fs.clone();
            let mut files = fs.files.lock();
            files.push(Arc::new(RAMINode {
                filesystem: strong,
                container: file,
            }))
        };
        add(RAMINodeContainer::Directory(vec![("small", 1), ("big", 2)]));
        add(RAMINodeContainer::File(IntMutex::new(
            "cat".as_bytes().to_vec(),
        )));
        let mut big_content = vec![];
        for _ in 0..4096 {
            big_content.push(b'd');
            big_content.push(b'o');
            big_content.push(b'g');
        }
        add(RAMINodeContainer::File(IntMutex::new(big_content)));
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

impl INode for RAMINode {
    fn lookup(&self, target: &str) -> Result<Arc<dyn INode>, &'static str> {
        let RAMINodeContainer::Directory(tree) = &self.container else {
            return Err("can't traverse a file");
        };
        for (name, inumber) in tree {
            if *name == target {
                return self
                    .filesystem
                    .get_inode(*inumber)
                    .ok_or("file found with an invalid inode?");
            }
        }
        Err("could not find file")
    }

    fn read_page(&self, physical_address: *mut u8, offset: usize) -> Result<(), &'static str> {
        if !offset.is_multiple_of(Arch::PAGE_SIZE) {
            return Err("given offset is not multiple of page size");
        }
        let RAMINodeContainer::File(content) = &self.container else {
            return Err("can't read the pages of a directory");
        };
        let content = content.lock();
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
        let RAMINodeContainer::File(content) = &self.container else {
            return Err("can't write to the pages of a directory");
        };
        let mut content = content.lock();
        if offset > content.len() {
            return Err("given offset is above file size");
        }
        let length = Arch::PAGE_SIZE.min(content.len() - offset);
        let hhdm = *HHDM_OFFSET.get().unwrap();
        for i in 0..length {
            let adjusted = (physical_address as usize) + hhdm + i;
            let adjusted = adjusted as *mut u8;
            unsafe { content[offset + i] = *adjusted }
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::physical_memory;
    use crate::physical_memory::HHDM_OFFSET;
    use crate::ramfs::RAMFilesystem;
    use crate::vfs::Filesystem;

    #[test_case]
    fn test_ramfs_small() {
        let fs = RAMFilesystem::new();
        let root = fs.get_root();
        let small = root.lookup("small").unwrap();
        let paddr = physical_memory::frame_alloc();
        let hhdm = *HHDM_OFFSET.get().unwrap();

        small.read_page(paddr as *mut u8, 0).unwrap();
        unsafe {
            assert!(*((paddr + hhdm + 0) as *const u8) == b'c');
            assert!(*((paddr + hhdm + 1) as *const u8) == b'a');
            assert!(*((paddr + hhdm + 2) as *const u8) == b't');
            *((paddr + hhdm + 0) as *mut u8) = b'b';
        }

        small.write_page(paddr as *const u8, 0).unwrap();
        unsafe { *((paddr + hhdm + 0) as *mut u8) = b'c' };

        small.read_page(paddr as *mut u8, 0).unwrap();
        unsafe {
            let b = *((paddr + hhdm + 0) as *const u8);
            assert!(b == b'b');
        }

        physical_memory::frame_dealloc(paddr);
    }

    #[test_case]
    fn test_ramfs_big() {
        let fs = RAMFilesystem::new();
        let root = fs.get_root();
        let big = root.lookup("big").unwrap();
        let paddr = physical_memory::frame_alloc();
        let hhdm = *HHDM_OFFSET.get().unwrap();

        big.read_page(paddr as *mut u8, 0).unwrap();
        unsafe {
            let d = *((paddr + hhdm + 0) as *const u8);
            let o = *((paddr + hhdm + 1) as *const u8);
            let g = *((paddr + hhdm + 2) as *const u8);
            assert!(d == b'd');
            assert!(o == b'o');
            assert!(g == b'g');
        }

        big.read_page(paddr as *mut u8, 4096).unwrap();
        unsafe {
            let o = *((paddr + hhdm + 0) as *const u8);
            let g = *((paddr + hhdm + 1) as *const u8);
            let d = *((paddr + hhdm + 2) as *const u8);
            assert!(o == b'o');
            assert!(g == b'g');
            assert!(d == b'd');
        }

        physical_memory::frame_dealloc(paddr);
    }
}
