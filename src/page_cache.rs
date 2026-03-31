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
    private_references: BTreeMap<usize, BTreeMap<usize, ()>>,
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
    fn create_file_page(
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

    fn create_anon_page(&mut self, key: &PageKey) -> Result<usize, &'static str> {
        let paddr = physical_memory::frame_alloc();
        self.map.insert(key.clone(), paddr);
        unsafe {
            let hhdm = HHDM_OFFSET.get().unwrap();
            ptr::write_bytes((paddr + hhdm) as *mut u8, 0, Arch::PAGE_SIZE);
        }
        Ok(paddr)
    }

    pub fn get_page(&mut self, key: &PageKey, cow: bool) -> Result<usize, &'static str> {
        let Some(paddr) = self.map.get(key) else {
            return match key {
                PageKey::File { inode_key, offset } => {
                    self.create_file_page(key, inode_key, offset)
                }
                key => self.create_anon_page(key),
            };
        };
        let old_paddr = *paddr;
        if !cow {
            return Ok(old_paddr);
        }
        let PageKey::Anonymous {
            process_id,
            virtual_address: _,
        } = key
        else {
            return Err("trying to cow a file backed cache");
        };
        let private_references = self
            .private_references
            .get_mut(&old_paddr)
            .ok_or("trying to cow a page with no private references")?;
        private_references.remove(process_id);
        let new_paddr = physical_memory::frame_alloc();
        unsafe { physical_memory::copy(old_paddr, new_paddr, Arch::PAGE_SIZE) };
        Ok(new_paddr)
    }

    pub fn add_private_reference(
        &mut self,
        vaddr: usize,
        parent_process: usize,
        child_process: usize,
    ) -> Result<(), &'static str> {
        let key = PageKey::Anonymous {
            process_id: parent_process,
            virtual_address: vaddr,
        };
        let paddr = self
            .map
            .get(&key)
            .ok_or("trying to add private reference to page that doesn't exist")?;
        let Some(private_references) = self.private_references.get_mut(paddr) else {
            let mut private_references = BTreeMap::new();
            private_references.insert(parent_process, ());
            private_references.insert(child_process, ());
            self.private_references.insert(*paddr, private_references);
            return Ok(());
        };
        private_references.insert(child_process, ());
        Ok(())
    }
}

pub static PAGE_CACHE: IntMutex<PageCache> = IntMutex::new(PageCache {
    map: BTreeMap::new(),
    private_references: BTreeMap::new(),
});
