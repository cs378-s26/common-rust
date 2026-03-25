use crate::{
    physical_memory, sync::IntMutex, vfs::INodeKey, virtual_memory_2::SHARED_ANONYMOUS_FILESYSTEM,
};
use alloc::collections::btree_map::BTreeMap;
pub struct PageCache {
    map: BTreeMap<PageKey, Page>,
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

struct Page {
    address: usize,
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
        self.map.insert(key.clone(), Page { address: paddr });
        Ok(paddr)
    }

    pub fn get_page(&mut self, key: &PageKey) -> Result<usize, &'static str> {
        match self.map.get_mut(key) {
            Some(page) => Ok(page.address),
            None => match key {
                PageKey::File { inode_key, offset } => self.get_file_page(key, inode_key, offset),
                _ => todo!(),
            },
        }
    }
}

pub static PAGE_CACHE: IntMutex<PageCache> = IntMutex::new(PageCache {
    map: BTreeMap::new(),
});
