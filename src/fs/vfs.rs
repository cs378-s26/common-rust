use alloc::{collections::btree_map::BTreeMap, sync::Arc};
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::sync::{IntMutex, MutexLike};

type INodeCache = BTreeMap<usize, BTreeMap<usize, Arc<dyn INode>>>;

pub struct VFS {
    filesystems: IntMutex<BTreeMap<usize, Arc<dyn Filesystem>>>,
    inode_cache: IntMutex<INodeCache>,
    filesystem_id_counter: AtomicUsize,
}

pub static VFS: VFS = VFS {
    filesystems: IntMutex::new(BTreeMap::new()),
    inode_cache: IntMutex::new(BTreeMap::new()),
    filesystem_id_counter: AtomicUsize::new(0),
};

impl VFS {
    pub fn mount(&self, filesystem: Arc<dyn Filesystem>) -> usize {
        let mut inode_cache = self.inode_cache.lock();
        let mut filesystems = self.filesystems.lock();
        let filesystem_id = self.filesystem_id_counter.fetch_add(1, Ordering::SeqCst);
        filesystems.insert(filesystem_id, filesystem);
        inode_cache.insert(filesystem_id, BTreeMap::new());
        filesystem_id
    }

    pub fn unmount(&self, filesystem_id: usize) {
        let mut inode_cache = self.inode_cache.lock();
        let mut filesystems = self.filesystems.lock();
        filesystems.remove(&filesystem_id);
        inode_cache.remove(&filesystem_id);
    }

    pub fn get_inode(&self, key: &INodeKey) -> Option<Arc<dyn INode>> {
        let mut inode_cache = self.inode_cache.lock();
        let map = inode_cache.get_mut(&key.filesystem_id)?;
        if let Some(inode) = map.get(&key.inumber) {
            return Some(Arc::clone(inode));
        }
        let inode = self
            .filesystems
            .lock()
            .get(&key.filesystem_id)?
            .get_inode(key.inumber)?;
        map.insert(key.inumber, Arc::clone(&inode));
        Some(inode)
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug)]
pub struct INodeKey {
    pub filesystem_id: usize,
    pub inumber: usize,
}

impl INodeKey {
    pub fn new(filesystem_id: usize, inumber: usize) -> Self {
        Self {
            filesystem_id,
            inumber,
        }
    }

    pub fn get_inode(&self) -> Option<Arc<dyn INode>> {
        VFS.get_inode(self)
    }
}

pub trait Filesystem: Send + Sync {
    fn get_root(&self) -> Arc<dyn INode>;
    fn get_inode(&self, inumber: usize) -> Option<Arc<dyn INode>>;
    fn create_inode(&self, inode: Arc<dyn INode>) -> usize; // TODO mutable reference?
    // delete_inode(inode)
}

pub trait INode: Send + Sync {
    // Files
    fn read_page(&self, physical_address: *mut u8, offset: usize) -> Result<(), &'static str>;
    fn write_page(&self, physical_address: *const u8, offset: usize) -> Result<(), &'static str>;

    // Directory
    fn lookup(&self, target: &str) -> Result<usize, &'static str>;
    fn add_entry(&self, target: &str, inumber: usize) -> Result<(), &'static str>;

    // Symlink
    // fn traverse() -> str
}

pub trait DirectoryTrait: Send + Sync {
    fn lookup(&self, target: &str) -> Result<usize, &'static str>;
    fn add_entry(&self, target: &str, inumber: usize) -> Result<(), &'static str>;
    // fn get_fs(&self) -> dyn Filesystem;
}
pub struct Directory<D: DirectoryTrait>(pub D); // rust type system moment

impl<D: DirectoryTrait> INode for Directory<D> {
    fn read_page(&self, _: *mut u8, _: usize) -> Result<(), &'static str> {
        Err("cannot read from directory")
    }
    fn write_page(&self, _: *const u8, _: usize) -> Result<(), &'static str> {
        Err("cannot write to directory")
    }
    fn lookup(&self, target: &str) -> Result<usize, &'static str> {
        self.0.lookup(target)
    }
    fn add_entry(&self, target: &str, inumber: usize) -> Result<(), &'static str> {
        self.0.add_entry(target, inumber)
    }
}

pub trait FileTrait: Send + Sync {
    // TODO is this just a block device?
    fn read_page(&self, physical_address: *mut u8, offset: usize) -> Result<(), &'static str>;
    fn write_page(&self, physical_address: *const u8, offset: usize) -> Result<(), &'static str>;
}
pub struct File<F: FileTrait>(pub F); // rust type system moment

impl<F: FileTrait> INode for File<F> {
    fn read_page(&self, physical_address: *mut u8, offset: usize) -> Result<(), &'static str> {
        self.0.read_page(physical_address, offset)
    }
    fn write_page(&self, physical_address: *const u8, offset: usize) -> Result<(), &'static str> {
        self.0.write_page(physical_address, offset)
    }
    fn lookup(&self, _: &str) -> Result<usize, &'static str> {
        Err("cannot perform lookup in file")
    }
    fn add_entry(&self, _: &str, _: usize) -> Result<(), &'static str> {
        Err("cannot add child to file")
    }
}
