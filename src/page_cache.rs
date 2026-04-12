use crate::{
    fs::vfs::INodeKey,
    physical_memory::{self},
    sync::IntMutex,
};
use alloc::collections::btree_map::BTreeMap;
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
        let inode = key
            .inode_key
            .get_inode()
            .map_err(|_| "could not get inode")?;
        inode
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
