use crate::{
    arch::{Arch, ArchTrait},
    physical_memory::{self, HHDM_OFFSET},
    sync::IntMutex,
    vfs::INodeKey,
    virtual_memory_2::SHARED_ANONYMOUS_FILESYSTEM,
};
use alloc::collections::btree_map::BTreeMap;
use core::ptr;
pub struct PageCache {
    map: BTreeMap<PageKey, usize>,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug)]
pub enum PageKey {
    Anonymous {
        process_id: usize,
        virtual_address: usize,
    },
    File {
        inode_key: INodeKey,
        offset: usize,
    },
}

impl PageCache {
    fn get_file_page(
        &mut self,
        key: &PageKey,
        inode_key: &INodeKey,
        offset: &usize,
    ) -> Result<usize, &'static str> {
        let paddr = physical_memory::frame_alloc();
        if inode_key.filesystem_id != SHARED_ANONYMOUS_FILESYSTEM {
            let inode = inode_key.get_inode().ok_or("file doesn't exist anymore")?;
            inode.read_page(paddr as *mut u8, *offset)?;
        }
        self.map.insert(key.clone(), paddr);
        Ok(paddr)
    }

    fn get_anon_page(&mut self, key: &PageKey) -> Result<usize, &'static str> {
        let paddr = physical_memory::frame_alloc();
        self.map.insert(key.clone(), paddr);
        unsafe {
            let hhdm = HHDM_OFFSET.get().unwrap();
            ptr::write_bytes((paddr + hhdm) as *mut u8, 0, Arch::PAGE_SIZE);
        }
        Ok(paddr)
    }

    pub fn get_page(&mut self, key: &PageKey) -> Result<usize, &'static str> {
        match self.map.get_mut(key) {
            Some(paddr) => Ok(*paddr),
            None => match key {
                PageKey::File { inode_key, offset } => self.get_file_page(key, inode_key, offset),
                key => self.get_anon_page(key),
            },
        }
    }
}

pub static PAGE_CACHE: IntMutex<PageCache> = IntMutex::new(PageCache {
    map: BTreeMap::new(),
});
