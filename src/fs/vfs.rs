use alloc::{collections::btree_map::BTreeMap, string::String, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::{
    devices::Device,
    sync::{IntMutex, MutexLike},
};

// TODO we probably don't want to cache on both the fs and the VFS level,
type INodeCache = BTreeMap<usize, BTreeMap<usize, Arc<dyn VNode>>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsError {
    MountAlreadyExists,
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
    mount_list: IntMutex<Vec<(Vec<&'static str>, usize)>>,
}

pub static VFS: VFS = VFS {
    filesystems: IntMutex::new(BTreeMap::new()),
    inode_cache: IntMutex::new(BTreeMap::new()),
    filesystem_id_counter: AtomicUsize::new(0),
    mount_list: IntMutex::new(Vec::new()),
};

impl VFS {
    // not a great mount yet, but this can be improved later for proper traversal.
    pub fn mount(
        &self,
        filesystem: Arc<dyn Filesystem>,
        path: &[&'static str],
    ) -> Result<usize, FsError> {
        // Lock everything down
        let mut mount_list = self.mount_list.lock();
        let mut inode_cache = self.inode_cache.lock();
        let mut filesystems = self.filesystems.lock();
        let filesystem_id = self.filesystem_id_counter.fetch_add(1, Ordering::SeqCst);

        // Do not mount here if something already exists
        if self.find_mount(path, &mount_list).is_some() {
            return Err(FsError::MountAlreadyExists);
        }

        // Create the mount
        let mut mount = (Vec::new(), filesystem_id);
        for p in path {
            mount.0.push(*p);
        }
        mount_list.push(mount);
        filesystem.set_filesystem_id(Some(filesystem_id));
        filesystems.insert(filesystem_id, filesystem);
        inode_cache.insert(filesystem_id, BTreeMap::new());
        Ok(filesystem_id)
    }

    pub fn unmount(&self, filesystem_id: usize) {
        // Lock everything down
        let mut mount_list = self.mount_list.lock();
        let mut inode_cache = self.inode_cache.lock();
        let mut filesystems = self.filesystems.lock();
        let mut mount_index = None;

        // Delete the mount
        for i in 0..mount_list.len() {
            if mount_list[i].1 == filesystem_id {
                mount_index = Some(i);
                break;
            }
        }
        if let Some(index) = mount_index {
            mount_list.swap_remove(index);
        } else {
            panic!("This should never happen");
        }
        if let Some(fs) = filesystems.remove(&filesystem_id) {
            fs.set_filesystem_id(None);
        }
        inode_cache.remove(&filesystem_id);
    }

    pub fn get_inode(&self, key: &INodeKey) -> Result<Arc<dyn VNode>, FsError> {
        let mut inode_cache = self.inode_cache.lock();
        let map = inode_cache
            .get_mut(&key.filesystem_id)
            .ok_or(FsError::NotFound)?;
        if let Some(inode) = map.get(&key.inumber) {
            return Ok(Arc::clone(inode));
        }
        let inode = self
            .filesystems
            .lock()
            .get(&key.filesystem_id)
            .ok_or(FsError::NotFound)?
            .get_inode(key.inumber)?;
        map.insert(key.inumber, Arc::clone(&inode));
        Ok(inode)
    }

    pub fn get_root(&self) -> Option<Arc<dyn VNode>> {
        let mount_list = self.mount_list.lock();
        let filesystems = self.filesystems.lock();
        let fs_index = self.find_mount(&["/"], &mount_list)?;
        let fs = filesystems.get(&fs_index)?;
        let root = fs.get_root();
        if let Ok(root) = root {
            return Some(Arc::clone(&root));
        }
        None
    }

    fn find_mount(
        &self,
        path: &[&'static str],
        mount_list: &Vec<(Vec<&'static str>, usize)>,
    ) -> Option<usize> {
        for mount in mount_list.iter() {
            if mount.0.len() != path.len() {
                continue;
            }
            let mut equals = true;
            for (i, p) in path.iter().enumerate() {
                if *p != mount.0[i] {
                    equals = false;
                    break;
                }
            }
            if !equals {
                continue;
            }
            return Some(mount.1);
        }
        None
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

    pub fn get_inode(&self) -> Result<Arc<dyn VNode>, FsError> {
        VFS.get_inode(self)
    }
}

pub trait Filesystem: Send + Sync {
    fn get_root(&self) -> Result<Arc<dyn VNode>, FsError>;
    fn get_inode(&self, inumber: usize) -> Result<Arc<dyn VNode>, FsError>;

    // these are for the VFS to get id's from the filesystem, particularly so a vnode's fs id can be recovered easily
    fn set_filesystem_id(&self, id: Option<usize>);
    fn get_filesystem_id(&self) -> Result<usize, FsError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum INodeType {
    File,
    Directory,
    // symlink or device, probably
    Other,
}

// dyn inode works as a typical vnode
pub trait VNode: Send + Sync {
    // Files
    fn get_inumber(&self) -> usize;

    fn get_type(&self) -> INodeType;

    // add default implementations for all these types so that filesystems don't need to
    // implement unnecessary functions, if they're a directory they just implement directory functions, etc
    fn read_page(&self, _physical_address: usize, _offset: usize) -> Result<usize, FsError> {
        Err(FsError::NotImplemented)
    }

    fn write_page(&self, _physical_address: usize, _offset: usize) -> Result<usize, FsError> {
        Err(FsError::NotImplemented)
    }

    // Directory
    fn lookup(&self, _target: &str) -> Result<Arc<dyn VNode>, FsError> {
        Err(FsError::NotImplemented)
    }

    fn add_entry(
        &self,
        _target: &str,
        _inumber: usize,
        _inode_type: INodeType,
    ) -> Result<(), FsError> {
        Err(FsError::NotImplemented)
    }

    // should only be implemented for directories
    fn create_child(&self, _name: &str, _inode_type: INodeType) -> Result<Arc<dyn VNode>, FsError> {
        Err(FsError::NotImplemented)
    }

    // should only be implemented for devices
    fn set_device(&self, _device: Arc<dyn Device>) -> Result<(), FsError> {
        Err(FsError::NotImplemented)
    }

    // file size, can be undefined
    fn size(&self) -> usize {
        0
    }

    // note: these really are not the main interface for vfs, reads and writes should be done through page cache,
    // this is for non-caching and convenience.
    fn read_unaligned(&self, _offset: usize, _buffer: &mut [u8]) -> Result<usize, FsError> {
        Err(FsError::NotImplemented)
    }

    fn write_unaligned(&self, _offset: usize, _buffer: &[u8]) -> Result<usize, FsError> {
        Err(FsError::NotImplemented)
    }

    // page cache needs to know filesystem id for InodeKey, this provides a way to get it. An Inode should store a reference to
    // whatever fs it's on. Maybe this could be done differently.
    fn get_inode_key(&self) -> Result<INodeKey, FsError> {
        Err(FsError::NotImplemented)
    }
    // Symlink
    // fn traverse() -> str
}
