use alloc::{
    collections::btree_map::BTreeMap,
    sync::{Arc, Weak},
    boxed::Box,
    string::{ToString, String}
};
use core::sync::atomic::{AtomicUsize, Ordering};

use spin::Once;

use crate::{
    fs::vfs::{Filesystem, FsError, INodeKey, INodeType, VNode, VFSDevice},
    sync::{IntMutex, MutexLike},
};

pub static DEV: Once<Arc<Dev>> = Once::new();

pub struct Dev {
    self_ref: Once<Weak<Self>>,
    counter: AtomicUsize,
    devices: IntMutex<BTreeMap<usize, Option<IntMutex<Arc<dyn VFSDevice>>>>>,
    fs_id: IntMutex<Option<usize>>,
}

pub struct DevINode {
    fs: Weak<Dev>,
    inumber: usize,
    device: IntMutex<Option<Arc<dyn VFSDevice>>>,
    //Devices can have children!
    children: IntMutex<BTreeMap<String, Arc<dyn VNode>>>,
}

impl Dev {
    pub fn new() -> Arc<Self> {
        let fs = Arc::new(Self {
            counter: AtomicUsize::new(1),
            devices: IntMutex::new(BTreeMap::new()),
            fs_id: IntMutex::new(None),
            self_ref: Once::new()
        });
        fs.self_ref.call_once(|| Arc::downgrade(&fs));
        fs
    }

    /* 
    fn alloc_inode_with_device(&self, inumber: usize, device: Option<Arc<IntMutex<Box<dyn VFSDevice>>>>) -> Result<Arc<dyn VNode>, FsError> {
        if let Some(device) = device.clone() {
            self.devices.lock().insert(inumber, Some(device));
        } else {
            self.devices.lock().insert(inumber, None);
        }
        Ok(Arc::new(DevINode {
            fs: self.self_ref.get().ok_or(FsError::ReadError)?.clone(),
            device: device,
            inumber,
            children: IntMutex::new(BTreeMap::new()),
        }))
    }
    */

    fn alloc_inode(&self, inumber: usize) -> Result<Arc<dyn VNode>, FsError> {
        let devices = self.devices.lock();
        let device = devices.get(&inumber).ok_or(FsError::ReadError)?;
        if let Some(device) = device {
            let arc_dev = device.lock();
            Ok(Arc::new(DevINode {
                fs: self.self_ref.get().ok_or(FsError::ReadError)?.clone(),
                device: IntMutex::new(Some(arc_dev.clone())),
                inumber,
                children: IntMutex::new(BTreeMap::new()),
            }))
        } else {
            Ok(Arc::new(DevINode {
                fs: self.self_ref.get().ok_or(FsError::ReadError)?.clone(),
                device: IntMutex::new(None),
                inumber,
                children: IntMutex::new(BTreeMap::new()),
            }))
        }
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

    fn create_child(&self, fname: &str, inode_type: INodeType) -> Result<Arc<dyn VNode>, FsError> {
        if self.inumber != 0 || inode_type != INodeType::Other {
            return Err(FsError::InvalidOperation);
        }
        let fs = self.fs.upgrade().ok_or(FsError::ReadError)?;
        let inumber = fs.counter.fetch_add(1, Ordering::SeqCst);
        fs.devices.lock().insert(inumber, None);
        let vnode = fs.alloc_inode(inumber);
        self.children.lock().insert(fname.to_string(), vnode.clone()?);
        vnode
    }

    fn set_device(&self, device: Arc<dyn VFSDevice>) -> Result<(), FsError> {
        self.device.lock().replace(device);
        Ok(())
    }

    // TODO: Device people, do these yourself. I don't know what your requirements are
    fn read_page(&self, _physical_address: usize, _offset: usize) -> Result<usize, FsError> {
        Err(FsError::NotImplemented)
    }

    fn write_page(&self, _physical_address: usize, _offset: usize) -> Result<usize, FsError> {
        Err(FsError::NotImplemented)
    }

    fn read_unaligned(&self, _offset: usize, buffer: &mut [u8]) -> Result<usize, FsError> {
        if let Some(device) = self.device.lock().as_ref() {
            device.read_unaligned(_offset, buffer)
        } else {
            Err(FsError::InvalidOperation)
        }
    }

    fn write_unaligned(&self, _offset: usize, buffer: &[u8]) -> Result<usize, FsError> {
        if let Some(device) = self.device.lock().as_ref() {
            device.write_unaligned(_offset, buffer)
        } else {
            Err(FsError::InvalidOperation)
        }
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

/*
* Allocates a device inode accessible by the user under `/dev/<fname>. `
*/
pub fn allocate_device_inode(fname : &str, device : Arc<dyn VFSDevice>) -> Result<(), FsError> {
    let dev = DEV.get().unwrap().clone();
    let root = dev.get_root()?;
    let inode: Arc<dyn VNode> = root.create_child(fname, INodeType::Other)?;
    inode.set_device(device)?;
    Ok(())
}
