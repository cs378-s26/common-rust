use crate::sync::IntMutex;
use crate::sync::MutexLike;
use alloc::collections::btree_map::BTreeMap;
use alloc::string::String;
use alloc::sync::Arc;
use core::sync::atomic::AtomicUsize;
use core::sync::atomic::Ordering;

// TODO we probably don't want to cache on both the fs and the VFS level,
type INodeCache = BTreeMap<usize, BTreeMap<usize, Arc<dyn INode>>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsError {
    NotFound,
    AlreadyExists,
    NoSpace,
    WriteError,
    ReadError,
    InvalidInput,
    InvalidOperation,
    NotImplemented,
    Corrupted(String),
    Other(String),
}

pub struct VFS {
    filesystems: IntMutex<BTreeMap<usize, Arc<dyn Filesystem>>>,
    inode_cache: IntMutex<INodeCache>,
    filesystem_id_counter: AtomicUsize,
    root: IntMutex<Option<Arc<dyn INode>>>,
}

pub static VFS: VFS = VFS {
    filesystems: IntMutex::new(BTreeMap::new()),
    inode_cache: IntMutex::new(BTreeMap::new()),
    filesystem_id_counter: AtomicUsize::new(0),
    root: IntMutex::new(None),
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

    pub fn get_root(&self) -> Option<Arc<dyn INode>> {
        let root = self.root.lock();
        if let Some(root) = &*root {
            return Some(Arc::clone(root));
        }
        None
    }

    pub fn set_root(&self, inode: Arc<dyn INode>) -> Result<(), FsError> {
        let mut root = self.root.lock();
        if root.is_some() {
            return Err(FsError::AlreadyExists);
        }
        *root = Some(inode);
        Ok(())
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
    // delete_inode(inode)
}

pub enum InodeType {
    File,
    Directory,
    // symlink or device, probably
    Other,
}

// dyn inode works as a typical vnode
pub trait INode: Send + Sync {
    // Files
    fn get_inumber(&self) -> usize;

    fn get_type(&self) -> INodeType;

    // add default implementations for all these types so that filesystems don't need to
    // implement unnecessary functions, if they're a directory they just implement directory functions, etc
    fn read_page(&self, physical_address: usize, offset: usize) -> Result<usize, FsError> {
        Err(FsError::NotImplemented)
    }

    fn write_page(&self, physical_address: usize, offset: usize) -> Result<usize, FsError> {
        Err(FsError::NotImplemented)
    }

    // Directory
    fn lookup(&self, target: &str) -> Result<Arc<dyn INode>, FsError> {
        Err(FsError::NotImplemented)
    }

    fn add_entry(&self, target: &str, inumber: usize) -> Result<(), FsError> {
        Err(FsError::NotImplemented)
    }

    // file size, can be undefined
    fn size(&self) -> usize {
        0
    }

    // Symlink
    // fn traverse() -> str
}
