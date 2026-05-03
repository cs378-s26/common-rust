use alloc::collections::btree_map::BTreeMap;

use crate::{fs::vfs::INodeKey, memory::physical_memory, sync::IntMutex};
pub struct PageCache {
    map: BTreeMap<PageKey, usize>,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug)]
pub struct PageKey {
    pub inode_key: INodeKey,
    pub offset: usize,
}

impl PageCache {
    fn get_file_page(&mut self, key: &PageKey) -> Result<usize, &'static str> {
        let paddr = physical_memory::frame_alloc();
        key.inode_key
            .get_inode()
            .map_err(|_| "could not get inode")?
            .read_page(paddr, key.offset)
            .map_err(|_| "could not read from file")?;
        self.map.insert(key.clone(), paddr);
        Ok(paddr)
    }

    pub fn get_page(&mut self, key: &PageKey) -> Result<usize, &'static str> {
        match self.map.get_mut(key) {
            Some(paddr) => Ok(*paddr),
            None => self.get_file_page(key),
        }
    }
}

pub static PAGE_CACHE: IntMutex<PageCache> = IntMutex::new(PageCache {
    map: BTreeMap::new(),
});

// this is the existing page cache to which I will be implementing modifications - MJ
