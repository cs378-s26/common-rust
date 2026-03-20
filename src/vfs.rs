use alloc::sync::Arc;
use alloc::sync::Weak;
use alloc::vec;
use alloc::vec::Vec;

use crate::arch::Arch;
use crate::arch::ArchTrait;
use crate::physical_memory::HHDM_OFFSET;
use crate::sync::IntMutex;
use crate::sync::MutexLike;

pub struct VFS {
    filesystems: Vec<Arc<dyn Filesystem>>,
}

pub trait Filesystem {
    fn get_root(&self) -> Weak<dyn INode>;
}

pub trait INode {
    fn lookup(&self, target: &str) -> Result<Weak<dyn INode>, &'static str>;
    fn read_page(&self, physical_address: *mut u8, offset: usize) -> Result<(), &'static str>;
    fn write_page(&self, physical_address: *const u8, offset: usize) -> Result<(), &'static str>;
}

pub struct RAMFilesystem {
    pub files: IntMutex<Vec<Arc<RAMINode>>>,
}

enum RAMINodeContainer {
    Directory(Vec<(&'static str, usize)>),
    File(IntMutex<Vec<u8>>),
}

pub struct RAMINode {
    filesystem: Arc<RAMFilesystem>,
    container: RAMINodeContainer,
}

impl RAMFilesystem {
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

    fn get_inode(&self, inumber: usize) -> Weak<dyn INode> {
        let file = self.files.lock()[inumber].clone();
        let file: Arc<dyn INode> = file;
        Arc::downgrade(&file)
    }
}

impl Filesystem for RAMFilesystem {
    fn get_root(&self) -> Weak<dyn INode> {
        self.get_inode(0)
    }
}

impl INode for RAMINode {
    fn lookup(&self, target: &str) -> Result<Weak<dyn INode>, &'static str> {
        match &self.container {
            RAMINodeContainer::Directory(tree) => {
                for (name, number) in tree {
                    if *name == target {
                        return Ok(self.filesystem.get_inode(*number));
                    }
                }
                Err("could not find file")
            }
            RAMINodeContainer::File(_) => Err("can't traverse a file"),
        }
    }

    fn read_page(&self, physical_address: *mut u8, offset: usize) -> Result<(), &'static str> {
        if !offset.is_multiple_of(Arch::PAGE_SIZE) {
            return Err("given offset is not multiple of page size");
        }
        match &self.container {
            RAMINodeContainer::Directory(_) => Err("can't read the pages of a directory"),
            RAMINodeContainer::File(content) => {
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
        }
    }

    fn write_page(&self, physical_address: *const u8, offset: usize) -> Result<(), &'static str> {
        match &self.container {
            RAMINodeContainer::Directory(_) => Err("can't read the pages of a directory"),
            RAMINodeContainer::File(content) => {
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
    }
}

#[cfg(test)]
mod test {
    use crate::physical_memory;
    use crate::physical_memory::HHDM_OFFSET;
    use crate::vfs::Filesystem;
    use crate::vfs::RAMFilesystem;

    #[test_case]
    fn test_ramfs_small() {
        let fs = RAMFilesystem::new();
        let root = match fs.get_root().upgrade() {
            Some(s) => s,
            None => panic!("could not unwrap root"),
        };
        let small = match root.lookup("small") {
            Ok(s) => match s.upgrade() {
                Some(s) => s,
                None => panic!("could not unwrap small"),
            },
            Err(e) => panic!("could not get small: {}", e),
        };
        let page = physical_memory::frame_alloc();
        match small.read_page(page as *mut u8, 0) {
            Ok(_) => {}
            Err(e) => panic!("could not read: {}", e),
        };
        let hhdm = *HHDM_OFFSET.get().unwrap();
        let c = unsafe { *((page + hhdm + 0) as *const u8) };
        let a = unsafe { *((page + hhdm + 1) as *const u8) };
        let t = unsafe { *((page + hhdm + 2) as *const u8) };
        assert!(c == b'c');
        assert!(a == b'a');
        assert!(t == b't');
        unsafe { *((page + hhdm + 0) as *mut u8) = b'b' };
        match small.write_page(page as *const u8, 0) {
            Ok(_) => {}
            Err(e) => panic!("could not write: {}", e),
        };
        unsafe { *((page + hhdm + 0) as *mut u8) = b'c' };
        match small.read_page(page as *mut u8, 0) {
            Ok(_) => {}
            Err(e) => panic!("could not read: {}", e),
        };
        let b = unsafe { *((page + hhdm + 0) as *const u8) };
        assert!(b == b'b');
        physical_memory::frame_dealloc(page as usize);
    }

    #[test_case]
    fn test_ramfs_big() {
        let fs = RAMFilesystem::new();
        let root = match fs.get_root().upgrade() {
            Some(s) => s,
            None => panic!("could not unwrap root"),
        };
        let big = match root.lookup("big") {
            Ok(s) => match s.upgrade() {
                Some(s) => s,
                None => panic!("could not unwrap big"),
            },
            Err(e) => panic!("could not get big: {}", e),
        };
        let page = physical_memory::frame_alloc();
        match big.read_page(page as *mut u8, 0) {
            Ok(_) => {}
            Err(e) => panic!("could not read: {}", e),
        };
        let hhdm = *HHDM_OFFSET.get().unwrap();
        let d = unsafe { *((page + hhdm + 0) as *const u8) };
        let o = unsafe { *((page + hhdm + 1) as *const u8) };
        let g = unsafe { *((page + hhdm + 2) as *const u8) };
        assert!(d == b'd');
        assert!(o == b'o');
        assert!(g == b'g');
        match big.read_page(page as *mut u8, 4096) {
            Ok(_) => {}
            Err(e) => panic!("could not read: {}", e),
        };
        let o = unsafe { *((page + hhdm + 0) as *const u8) };
        let g = unsafe { *((page + hhdm + 1) as *const u8) };
        let d = unsafe { *((page + hhdm + 2) as *const u8) };
        assert!(o == b'o');
        assert!(g == b'g');
        assert!(d == b'd');
        physical_memory::frame_dealloc(page as usize);
    }
}
