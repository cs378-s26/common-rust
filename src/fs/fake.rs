use core::ptr;
use core::sync::atomic::{AtomicUsize, Ordering};

use alloc::collections::btree_map::BTreeMap;
use alloc::sync::Arc;
use alloc::sync::Weak;
use spin::Once;

use crate::arch::Arch;
use crate::arch::ArchTrait;
use crate::fs::vfs::INodeKey;
use crate::fs::vfs::{Filesystem, FsError, INodeType, VNode};
use crate::physical_memory::HHDM_OFFSET;
use crate::sync::{IntMutex, MutexLike};

pub static FAKE: Once<Arc<Fake>> = Once::new();

pub struct Fake {
    self_ref: Once<Weak<Self>>,
    counter: AtomicUsize,
    active: IntMutex<BTreeMap<usize, ()>>,
    fs_id: IntMutex<Option<usize>>,
}

pub struct FakeINode {
    fs: Weak<Fake>,
    inumber: usize,
}

impl Fake {
    pub fn new() -> Arc<Self> {
        let fs = Arc::new(Self {
            counter: AtomicUsize::new(1),
            active: IntMutex::new(BTreeMap::new()),
            fs_id: IntMutex::new(None),
            self_ref: Once::new(),
        });
        fs.self_ref.call_once(|| Arc::downgrade(&fs));
        fs
    }

    fn alloc_inode(&self, inumber: usize) -> Result<Arc<dyn VNode>, FsError> {
        Ok(Arc::new(FakeINode {
            fs: self.self_ref.get().ok_or(FsError::ReadError)?.clone(),
            inumber,
        }))
    }
}

impl Filesystem for Fake {
    fn get_root(&self) -> Result<Arc<dyn VNode>, FsError> {
        self.get_inode(0)
    }

    fn get_inode(&self, inumber: usize) -> Result<Arc<dyn VNode>, FsError> {
        if inumber == 0 || self.active.lock().contains_key(&inumber) {
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

impl VNode for FakeINode {
    fn get_inumber(&self) -> usize {
        self.inumber
    }

    fn get_type(&self) -> INodeType {
        if self.inumber == 0 {
            INodeType::Directory
        } else {
            INodeType::File
        }
    }

    fn create_child(&self, _: &str, inode_type: INodeType) -> Result<Arc<dyn VNode>, FsError> {
        if self.inumber != 0 || inode_type != INodeType::File {
            return Err(FsError::InvalidOperation);
        }
        let fs = self.fs.upgrade().ok_or(FsError::ReadError)?;
        let inumber = fs.counter.fetch_add(1, Ordering::SeqCst);
        fs.active.lock().insert(inumber, ());
        fs.alloc_inode(inumber)
    }

    fn read_page(&self, physical_address: usize, _offset: usize) -> Result<usize, FsError> {
        let hhdm = HHDM_OFFSET.get().unwrap();
        unsafe { ptr::write_bytes((physical_address + hhdm) as *mut u8, 0, Arch::PAGE_SIZE) };
        Ok(Arch::PAGE_SIZE)
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

pub fn create_fake_file() -> Result<INodeKey, &'static str> {
    FAKE.get()
        .ok_or("could not get fake filesystem")?
        .get_root()
        .map_err(|_| "could not get root of fake filesystem")?
        .create_child("", INodeType::File)
        .map_err(|_| "could not create child of fake filesystem")?
        .get_inode_key()
        .map_err(|_| "could not get inode key of fake filesystem")
}
