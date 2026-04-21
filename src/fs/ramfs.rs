use alloc::{
    collections::btree_map::BTreeMap,
    string::String,
    sync::{Arc, Weak},
    vec::Vec,
};

use spin::Once;

use crate::{
    arch::{Arch, ArchTrait},
    fs::vfs::{Filesystem, FsError, INodeKey, INodeType, VNode},
    memory::physical_memory::HHDM_OFFSET,
    sync::{IntMutex, MutexLike},
};

pub struct RamFilesystem {
    self_ref: Once<Weak<Self>>,
    inodes: IntMutex<Vec<Arc<RamInode>>>,
    fs_id: IntMutex<Option<usize>>,
}

enum RamInodeKind {
    File {
        data: IntMutex<Vec<u8>>,
    },
    Dir {
        entries: IntMutex<BTreeMap<String, usize>>,
    },
}

pub struct RamInode {
    number: usize,
    kind: RamInodeKind,
    fs: Weak<RamFilesystem>,
}

impl RamFilesystem {
    pub fn new() -> Arc<Self> {
        let fs = Arc::new(Self {
            self_ref: Once::new(),
            inodes: IntMutex::new(Vec::new()),
            fs_id: IntMutex::new(None),
        });
        fs.self_ref.call_once(|| Arc::downgrade(&fs));

        // inode 0 = root directory
        let root = Arc::new(RamInode {
            number: 0,
            kind: RamInodeKind::Dir {
                entries: IntMutex::new(BTreeMap::new()),
            },
            fs: fs.self_ref.get().unwrap().clone(),
        });
        fs.inodes.lock().push(root);
        fs
    }

    fn alloc_inode(&self, kind: RamInodeKind) -> Arc<RamInode> {
        let mut inodes = self.inodes.lock();
        let number = inodes.len();
        let inode = Arc::new(RamInode {
            number,
            kind,
            fs: self.self_ref.get().unwrap().clone(),
        });
        inodes.push(inode.clone());
        inode
    }
}

impl Filesystem for RamFilesystem {
    fn get_root(&self) -> Result<Arc<dyn VNode>, FsError> {
        self.get_inode(0)
    }

    fn get_inode(&self, inumber: usize) -> Result<Arc<dyn VNode>, FsError> {
        let inodes = self.inodes.lock();
        let inode = inodes.get(inumber).ok_or(FsError::NotFound)?;
        Ok(inode.clone())
    }

    fn set_filesystem_id(&self, id: Option<usize>) {
        *self.fs_id.lock() = id;
    }

    fn get_filesystem_id(&self) -> Result<usize, FsError> {
        self.fs_id.lock().ok_or(FsError::NotFound)
    }
}

impl RamInode {
    fn fs_id(&self) -> Result<usize, FsError> {
        self.fs
            .upgrade()
            .ok_or(FsError::NotFound)?
            .get_filesystem_id()
    }

    fn as_dir(&self) -> Result<&IntMutex<BTreeMap<String, usize>>, FsError> {
        match &self.kind {
            RamInodeKind::Dir { entries } => Ok(entries),
            _ => Err(FsError::InvalidOperation),
        }
    }

    fn as_file(&self) -> Result<&IntMutex<Vec<u8>>, FsError> {
        match &self.kind {
            RamInodeKind::File { data } => Ok(data),
            _ => Err(FsError::InvalidOperation),
        }
    }
}

impl VNode for RamInode {
    fn get_inumber(&self) -> usize {
        self.number
    }

    fn get_type(&self) -> INodeType {
        match self.kind {
            RamInodeKind::File { .. } => INodeType::File,
            RamInodeKind::Dir { .. } => INodeType::Directory,
        }
    }

    fn read_page(&self, physical_address: usize, offset: usize) -> Result<usize, FsError> {
        let virt_addr = physical_address + HHDM_OFFSET.get().expect("HHDM_OFFSET not set");

        // Safety: limine provides us with hhdm mappings of all physical memory
        let page_buf =
            unsafe { core::slice::from_raw_parts_mut(virt_addr as *mut u8, Arch::PAGE_SIZE) };

        let data = self.as_file()?.lock();
        page_buf.fill(0);

        if offset >= data.len() {
            return Ok(0);
        }

        let len = core::cmp::min(Arch::PAGE_SIZE, data.len() - offset);
        page_buf[..len].copy_from_slice(&data[offset..offset + len]);
        Ok(len)
    }

    fn write_page(&self, physical_address: usize, offset: usize) -> Result<usize, FsError> {
        let virt_addr = physical_address + HHDM_OFFSET.get().expect("HHDM_OFFSET not set");

        // Safety: limine provides us with hhdm mappings of all physical memory
        let page = unsafe { core::slice::from_raw_parts(virt_addr as *const u8, Arch::PAGE_SIZE) };

        let mut data = self.as_file()?.lock();
        let end = offset.saturating_add(Arch::PAGE_SIZE);
        if end > data.len() {
            data.resize(end, 0);
        }

        data[offset..end].copy_from_slice(page);
        Ok(Arch::PAGE_SIZE)
    }

    fn lookup(&self, target: &str) -> Result<Arc<dyn VNode>, FsError> {
        let entries = self.as_dir()?;
        let guard = entries.lock();
        let inumber = guard.get(target).ok_or(FsError::NotFound)?;
        let fs = self.fs.upgrade().ok_or(FsError::NotFound)?;
        fs.get_inode(*inumber)
    }

    fn add_entry(
        &self,
        target: &str,
        inumber: usize,
        _inode_type: INodeType,
    ) -> Result<(), FsError> {
        let entries = self.as_dir()?;
        let mut guard = entries.lock();
        if guard.contains_key(target) {
            return Err(FsError::AlreadyExists);
        }
        guard.insert(target.into(), inumber);
        Ok(())
    }

    fn create_child(&self, name: &str, inode_type: INodeType) -> Result<Arc<dyn VNode>, FsError> {
        let fs = self.fs.upgrade().ok_or(FsError::NotFound)?;
        let new_kind = match inode_type {
            INodeType::File => RamInodeKind::File {
                data: IntMutex::new(Vec::new()),
            },
            INodeType::Directory => RamInodeKind::Dir {
                entries: IntMutex::new(BTreeMap::new()),
            },
            INodeType::Other => return Err(FsError::InvalidOperation),
        };
        let inode = fs.alloc_inode(new_kind);
        self.add_entry(name, inode.number, inode_type)?;
        Ok(inode)
    }

    fn size(&self) -> usize {
        match &self.kind {
            RamInodeKind::File { data } => data.lock().len(),
            RamInodeKind::Dir { entries } => entries.lock().len(),
        }
    }

    fn read_unaligned(&self, offset: usize, buffer: &mut [u8]) -> Result<usize, FsError> {
        let data = self.as_file()?.lock();
        if offset >= data.len() {
            return Ok(0);
        }
        let len = core::cmp::min(buffer.len(), data.len() - offset);
        buffer[..len].copy_from_slice(&data[offset..offset + len]);
        Ok(len)
    }

    fn write_unaligned(&self, offset: usize, buffer: &[u8]) -> Result<usize, FsError> {
        let mut data = self.as_file()?.lock();
        if offset + buffer.len() > data.len() {
            data.resize(offset + buffer.len(), 0);
        }
        data[offset..offset + buffer.len()].copy_from_slice(buffer);
        Ok(buffer.len())
    }

    fn get_inode_key(&self) -> Result<INodeKey, FsError> {
        Ok(INodeKey {
            filesystem_id: self.fs_id()?,
            inumber: self.number,
        })
    }
}
