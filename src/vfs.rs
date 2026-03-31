use crate::sync::IntMutex;
use crate::sync::MutexLike;
use alloc::collections::btree_map::BTreeMap;
use alloc::sync::Arc;
use core::sync::atomic::AtomicUsize;
use core::sync::atomic::Ordering;

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
    fn create_inode(&self) -> usize;
    // delete_inode(inode)
}

pub trait INode: Send + Sync {
    // Files
    fn read_page(&self, physical_address: *mut u8, offset: usize) -> Result<(), &'static str>;
    fn write_page(&self, physical_address: *const u8, offset: usize) -> Result<(), &'static str>;

    // Directory
    fn lookup(&self, target: &str) -> Result<Arc<dyn INode>, &'static str>;
    // add_direntry(name, inode, file_type)

    // Symlink
    // fn traverse() -> str
}
