// TODO: integrate this with MJ's VMM (especially the shadowing)
// Currently, very divergent.

// TODO: use blocking locks

// TODO: consider locking stuff more effeciently

// TODO: minimize unwraps

use crate::arch::{Arch, ArchTrait};
use crate::free::FreeSet;
use crate::physical_memory;
use crate::vfs::INodeKey;
use crate::virtual_memory::PagingOptions;
use alloc::boxed::Box;
use alloc::collections::btree_map::BTreeMap;
use intrusive_collections::{Bound::Included, RBTreeLink};
use intrusive_collections::{KeyAdapter, RBTree, intrusive_adapter};
use spin::{Mutex, Once};

struct Mapping {
    file: Option<(INodeKey, usize, Option<usize>)>,
    length: usize,
    shared: bool,
    base: usize,
    link: RBTreeLink,
}

intrusive_adapter!(MappingAdapter = Box<Mapping>: Mapping { link => RBTreeLink });
impl<'a> KeyAdapter<'a> for MappingAdapter {
    type Key = usize;
    fn get_key(&self, x: &'a Mapping) -> usize {
        x.base
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum PageKey {
    Anonymous { process_id: usize, offset: usize },
    File { inode_key: INodeKey, offset: usize },
}

struct Page {
    address: usize,
}

struct PageCache {
    map: BTreeMap<PageKey, Page>,
}

impl PageCache {
    fn get_page(&mut self, key: PageKey) -> Option<usize> {
        match self.map.get(&key) {
            Some(p) => Some(p.address),
            None => match &key {
                PageKey::Anonymous { process_id, offset } => None,
                PageKey::File { inode_key, offset } => {
                    let inode = inode_key.get_inode().unwrap();
                    let frame = physical_memory::frame_alloc();
                    inode.read_page(frame as *mut u8, *offset).unwrap();
                    self.map.insert(key, Page { address: frame });
                    Some(frame)
                }
            },
        }
    }
}

pub struct VirtualMemory {
    free_set: FreeSet,
    active_set: RBTree<MappingAdapter>,
}

static PAGE_CACHE: Mutex<PageCache> = Mutex::new(PageCache {
    map: BTreeMap::new(),
});

impl VirtualMemory {
    pub fn new() -> Self {
        let mut set = FreeSet::new();
        set.add_range(0x100000, 0x8000_0000_0000_0000 - 0x100000)
            .unwrap();
        Self {
            free_set: set,
            active_set: RBTree::new(MappingAdapter::new()),
        }
    }

    // four cases, (file private), (file shared), (anon private), (anon shared)
    pub fn handle_page_fault(&self, address: usize) {
        assert!(address.is_multiple_of(Arch::PAGE_SIZE));
        let mapping = self
            .active_set
            .upper_bound(Included(&address))
            .get()
            .unwrap();
        let length = mapping.length;
        let shared = mapping.shared;
        // let (inode_key, offset, file_length) = mapping.file.unwrap();
        let (inode_key, offset, file_length) = mapping.file.as_ref().unwrap();
        let physical_address = PAGE_CACHE
            .lock()
            .get_page(PageKey::File {
                inode_key: inode_key.clone(),
                offset: address - mapping.base + offset,
            })
            .unwrap();

        // TODO: use an address space that depends on something
        Arch::virtual_map(
            Arch::get_address_space(),
            address as u64,
            physical_address as u64,
            PagingOptions::PRESENT | PagingOptions::WRITABLE | PagingOptions::CACHEABLE,
        );
    }

    pub fn mmap(
        &mut self,
        file: Option<(INodeKey, usize, Option<usize>)>,
        length: usize,
        shared: bool,
        preferred_base: Option<usize>,
    ) -> Result<usize, &'static str> {
        if !length.is_multiple_of(Arch::PAGE_SIZE) {
            return Err("map length must be aligned to page boundary");
        }
        if let Some((_, offset, Some(file_length))) = file {
            if !offset.is_multiple_of(Arch::PAGE_SIZE) {
                return Err("file offset must be aligned to page boundary");
            }
            if file_length > length {
                return Err("file length is bigger than length of map");
            }
        }
        let base: usize;
        if let Some(preferred_base) = preferred_base {
            match self.free_set.remove_range_by_base(preferred_base, length) {
                Ok(_) => base = preferred_base,
                Err(e) => return Err(e),
            }
        } else {
            match self.free_set.remove_range_by_length(length) {
                Ok(b) => base = b,
                Err(e) => return Err(e),
            }
        }
        self.active_set.insert(Box::new(Mapping {
            file,
            length,
            shared,
            base,
            link: RBTreeLink::new(),
        }));
        Ok(base)
    }
    // TODO: munmap
}

pub static VMM: Once<Mutex<VirtualMemory>> = Once::new();
