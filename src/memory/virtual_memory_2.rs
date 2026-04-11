use alloc::boxed::Box;
use core::sync::atomic::{AtomicUsize, Ordering};

use intrusive_collections::{KeyAdapter, RBTree, RBTreeLink, intrusive_adapter};
use spin::Once;

use crate::{
    arch::{Arch, ArchTrait},
    fs::vfs::INodeKey,
    memory::{freeset::FreeSet, physical_memory},
    sync::{IntMutex, MutexLike},
};

const SHARED_ANONYMOUS_FILESYSTEM: usize = usize::MAX;
static SHARED_ANONYMOUS_COUNTER: AtomicUsize = AtomicUsize::new(0);
const USERSPACE_START: usize = 0x10000;
const USERSPACE_END: usize = 0x8000_0000_0000_0000;
static LIMINE_PAGE_TABLE: Once<usize> = Once::new();

struct Mapping {
    #[allow(unused)]
    file: Option<(INodeKey, usize, Option<usize>)>,
    #[allow(unused)]
    length: usize,
    #[allow(unused)]
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

pub struct VirtualMemory {
    free_set: IntMutex<FreeSet>,
    active_set: IntMutex<RBTree<MappingAdapter>>,
    page_table: usize,
}

impl VirtualMemory {
    pub fn init() {
        let limine_page_table =
            LIMINE_PAGE_TABLE.call_once(|| Arch::get_user_address_space() as usize);
        assert!(limine_page_table.is_multiple_of(Arch::PAGE_SIZE));
    }

    pub fn mmap(
        &self,
        file: Option<(INodeKey, usize, Option<usize>)>,
        length: usize,
        shared: bool,
        preferred_base: Option<usize>,
    ) -> Result<usize, &'static str> {
        let mut file = file;
        if !length.is_multiple_of(Arch::PAGE_SIZE) {
            return Err("map length must be aligned to page boundary");
        }
        if let Some((_, offset, file_length)) = file {
            if !offset.is_multiple_of(Arch::PAGE_SIZE) {
                return Err("file offset must be aligned to page boundary");
            }
            if file_length.is_some() && shared {
                return Err(
                    "we do not allow partial file maps if shared (this feature only affects the ELF loader)",
                );
            }
            if let Some(file_length) = file_length
                && file_length > length
            {
                return Err("file length is bigger than length of map");
            }
        }
        if file.is_none() && shared {
            let inode_key = INodeKey::new(
                SHARED_ANONYMOUS_FILESYSTEM,
                SHARED_ANONYMOUS_COUNTER.fetch_add(1, Ordering::SeqCst),
            );
            file = Some((inode_key, 0, None));
        }
        let base: usize;
        let mut free_set = self.free_set.lock();
        if let Some(preferred_base) = preferred_base {
            free_set.remove_range_by_base(preferred_base, length)?;
            base = preferred_base;
        } else {
            base = free_set.remove_range_by_length(length)?;
        }
        let mut active_set = self.active_set.lock();
        active_set.insert(Box::new(Mapping {
            file,
            length,
            shared,
            base,
            link: RBTreeLink::new(),
        }));
        Ok(base)
    }

    pub fn new() -> Self {
        let mut set = FreeSet::new();
        set.add_range(USERSPACE_START, USERSPACE_END - USERSPACE_START)
            .unwrap();
        Self {
            free_set: IntMutex::new(set),
            active_set: IntMutex::new(RBTree::new(MappingAdapter::new())),
            page_table: Self::new_page_table(),
        }
    }

    pub fn get_page_table(&self) -> usize {
        self.page_table
    }

    pub fn get_limine_page_table() -> usize {
        *LIMINE_PAGE_TABLE.get().unwrap()
    }
}

impl VirtualMemory {
    fn new_page_table() -> usize {
        let page_table = physical_memory::frame_alloc();
        unsafe {
            physical_memory::copy(Self::get_limine_page_table(), page_table, Arch::PAGE_SIZE)
        };
        page_table
    }
}

impl Default for VirtualMemory {
    fn default() -> Self {
        Self::new()
    }
}
