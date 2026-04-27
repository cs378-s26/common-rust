use alloc::{
    collections::btree_map::BTreeMap,
    sync::{Arc, Weak},
};
use core::sync::atomic::{AtomicUsize, Ordering};

use spin::Once;

use crate::{
    devices::Device,
    fs::vfs::{Filesystem, FsError, INodeKey, INodeType, VNode},
    sync::{IntMutex, MutexLike},
};

pub static DEV: Once<Arc<Dev>> = Once::new();

pub struct Dev {
    self_ref: Once<Weak<Self>>,
    counter: AtomicUsize,
    devices: IntMutex<BTreeMap<usize, Option<Arc<dyn Device>>>>,
    fs_id: IntMutex<Option<usize>>,
}

pub struct DevINode {
    fs: Weak<Dev>,
    inumber: usize,
    device: IntMutex<Option<Arc<dyn Device>>>,
}

impl Dev {
    pub fn new() -> Arc<Self> {
        let fs = Arc::new(Self {
            counter: AtomicUsize::new(1),
            devices: IntMutex::new(BTreeMap::new()),
            fs_id: IntMutex::new(None),
            self_ref: Once::new(),
        });
        fs.self_ref.call_once(|| Arc::downgrade(&fs));
        fs
    }

    fn alloc_inode(&self, inumber: usize) -> Result<Arc<dyn VNode>, FsError> {
        let devices = self.devices.lock();
        let device = devices.get(&inumber).ok_or(FsError::ReadError)?;
        let cloned_device;
        if let Some(device) = device {
            cloned_device = Some(Arc::clone(device));
        } else {
            cloned_device = None;
        }
        Ok(Arc::new(DevINode {
            fs: self.self_ref.get().ok_or(FsError::ReadError)?.clone(),
            device: IntMutex::new(cloned_device),
            inumber,
        }))
    }
}

impl Filesystem for Dev {
    fn get_root(&self) -> Result<Arc<dyn VNode>, FsError> {
        self.get_inode(0)
    }

    fn get_inode(&self, inumber: usize) -> Result<Arc<dyn VNode>, FsError> {
        if inumber == 0 || self.devices.lock().contains_key(&inumber) {
            self.alloc_inode(inumber)
        } else {
            Err(FsError::NotFound)
        }
    }

    fn set_filesystem_id(&self, id: Option<usize>) {
        *self.fs_id.lock() = id;
    }

    fn get_filesystem_id(&self) -> Result<usize, FsError> {
        self.fs_id.lock().ok_or(FsError::NotFound)
    }
}

impl VNode for DevINode {
    fn get_inumber(&self) -> usize {
        self.inumber
    }

    fn get_type(&self) -> INodeType {
        if self.inumber == 0 {
            INodeType::Directory
        } else {
            INodeType::Other
        }
    }

    fn create_child(&self, _: &str, inode_type: INodeType) -> Result<Arc<dyn VNode>, FsError> {
        if self.inumber != 0 || inode_type != INodeType::Other {
            return Err(FsError::InvalidOperation);
        }
        let fs = self.fs.upgrade().ok_or(FsError::ReadError)?;
        let inumber = fs.counter.fetch_add(1, Ordering::SeqCst);
        fs.devices.lock().insert(inumber, None);
        fs.alloc_inode(inumber)
    }

    fn set_device(&self, device: Arc<dyn Device>) -> Result<(), FsError> {
        *self.device.lock() = Some(device);
        Ok(())
    }

    // TODO: Device people, do these yourself. I don't know what your requirements are
    fn read_page(&self, _physical_address: usize, _offset: usize) -> Result<usize, FsError> {
        Err(FsError::NotImplemented)
    }

    fn write_page(&self, _physical_address: usize, _offset: usize) -> Result<usize, FsError> {
        Err(FsError::NotImplemented)
    }

    fn get_inode_key(&self) -> Result<INodeKey, FsError> {
        let result = INodeKey {
            filesystem_id: self
                .fs
                .upgrade()
                .ok_or(FsError::ReadError)?
                .fs_id
                .lock()
                .ok_or(FsError::ReadError)?,
            inumber: self.inumber,
        };
        Ok(result)
    }
}
