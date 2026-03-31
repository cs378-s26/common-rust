use crate::{
    arch::{Arch, ArchTrait},
    freeset::FreeSet,
    page_cache::{PAGE_CACHE, PageKey},
    physical_memory,
    sync::{IntMutex, MutexLike},
    vfs::INodeKey,
    virtual_memory::{PageFaultConditions, PagingOptions},
};
use alloc::{boxed::Box, collections::btree_map::BTreeMap};
use core::sync::atomic::{AtomicUsize, Ordering};
use intrusive_collections::{Bound, KeyAdapter, RBTree, RBTreeLink, intrusive_adapter};
use spin::Once;

pub const SHARED_ANONYMOUS_FILESYSTEM: usize = usize::MAX;
static SHARED_ANONYMOUS_COUNTER: AtomicUsize = AtomicUsize::new(0);
const USERSPACE_START: usize = 0x10000;
const USERSPACE_END: usize = 0x8000_0000_0000_0000;
static LIMINE_PAGE_TABLE: Once<usize> = Once::new();

struct Mapping {
    file: Option<(INodeKey, usize, Option<usize>)>,
    length: usize,
    shared: bool,
    base: usize,
    link: RBTreeLink,
    private_pages: IntMutex<BTreeMap<usize, ()>>,
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
            private_pages: IntMutex::new(BTreeMap::new()),
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

impl Clone for VirtualMemory {
    // TODO: Make this less greedy with the locks
    fn clone(&self) -> Self {
        let mut page_cache = PAGE_CACHE.lock();
        let child_page_table = Self::new_page_table();

        let parent_free_set = self.free_set.lock();
        let child_free_set = parent_free_set.clone();

        let parent_active_set = self.active_set.lock();
        let mut child_active_set = RBTree::new(MappingAdapter::new());

        for parent_mapping in parent_active_set.iter() {
            let mut child_private_pages = BTreeMap::new();
            for (vaddr, _) in parent_mapping.private_pages.lock().iter() {
                page_cache
                    .add_private_reference(*vaddr, self.page_table, child_page_table)
                    .unwrap();
                child_private_pages.insert(*vaddr, ());
                // TODO: Just change cr3 to flush entire TLB
                self.invlpg(*vaddr);
            }
            let child_mapping = Mapping {
                file: parent_mapping.file.clone(),
                link: RBTreeLink::new(),
                private_pages: IntMutex::new(child_private_pages),
                ..*parent_mapping
            };
            child_active_set.insert(Box::new(child_mapping));
        }
        Self {
            active_set: IntMutex::new(child_active_set),
            free_set: IntMutex::new(child_free_set),
            page_table: child_page_table,
        }
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

impl VirtualMemory {
    // TODO: Implement per mapping locking (needed for multithreaded
    // processes).

    // TODO: Let's say we have two threads (X, Y) in a process. X
    // faults, and soon Y faults on the same address. X acquires the
    // mapping lock, and Y waits for X. X correctly vmaps such that X
    // and Y are satisfied. X releases lock. Y should now know that it
    // doesn't need to map anything. This hasn't been implemented.
    pub fn handle_page_fault(
        &self,
        cause: &PageFaultConditions,
        address: usize,
    ) -> Result<(), &'static str> {
        let vaddr = address & !(Arch::PAGE_SIZE - 1);
        let active_set = self.active_set.lock();
        let mapping = active_set
            .upper_bound(Bound::Included(&vaddr))
            .get()
            .ok_or("nothing is mapped")?;
        if mapping.base + mapping.length <= vaddr {
            return Err("out of mapped range");
        }
        // TODO: This is too broad with how it deals with TLB
        // shootdowns. It does a shootdown, even when we are doing a
        // read->write promotion, which is not needed.
        if cause.contains(PageFaultConditions::PRESENT) {
            self.invlpg(vaddr);
        }
        if let Some((inode_key, offset, file_length)) = mapping.file.as_ref() {
            if mapping.shared {
                self.handle_file_shared(vaddr, mapping, inode_key, offset)
            } else {
                self.handle_file_private(cause, vaddr, mapping, inode_key, offset, file_length)
            }
        } else {
            assert!(!mapping.shared);
            self.handle_anon_private(cause, vaddr, mapping)
        }
    }

    fn handle_file_shared(
        &self,
        vaddr: usize,
        mapping: &Mapping,
        inode_key: &INodeKey,
        offset: &usize,
    ) -> Result<(), &'static str> {
        assert!(vaddr.is_multiple_of(Arch::PAGE_SIZE));
        let key = PageKey::File {
            inode_key: inode_key.clone(),
            offset: vaddr - mapping.base + offset,
        };
        let paddr = PAGE_CACHE.lock().get_page(&key, false)?;
        self.vmap_write(vaddr, paddr);
        Ok(())
    }

    fn handle_anon_private(
        &self,
        cause: &PageFaultConditions,
        vaddr: usize,
        mapping: &Mapping,
    ) -> Result<(), &'static str> {
        assert!(vaddr.is_multiple_of(Arch::PAGE_SIZE));
        if self.already_has(cause, vaddr, mapping)? {
            return Ok(());
        }
        let key = PageKey::Anonymous {
            process_id: self.page_table,
            virtual_address: vaddr,
        };
        let paddr = PAGE_CACHE.lock().get_page(&key, false)?;
        mapping.private_pages.lock().insert(vaddr, ());
        self.vmap_write(vaddr, paddr);
        Ok(())
    }

    fn handle_file_private(
        &self,
        cause: &PageFaultConditions,
        vaddr: usize,
        mapping: &Mapping,
        inode_key: &INodeKey,
        offset: &usize,
        file_length: &Option<usize>,
    ) -> Result<(), &'static str> {
        assert!(vaddr.is_multiple_of(Arch::PAGE_SIZE));
        if self.already_has(cause, vaddr, mapping)? {
            return Ok(());
        }
        if let Some(file_length) = file_length
            && self.handle_file_private_partial(vaddr, mapping, inode_key, offset, file_length)?
        {
            return Ok(());
        }
        let shared_key = PageKey::File {
            inode_key: inode_key.clone(),
            offset: *offset,
        };
        let shared_paddr = PAGE_CACHE.lock().get_page(&shared_key, false)?;
        if !cause.contains(PageFaultConditions::WRITE) {
            self.vmap_read(vaddr, shared_paddr);
            return Ok(());
        }
        let private_key = PageKey::Anonymous {
            process_id: self.page_table,
            virtual_address: vaddr,
        };
        let private_paddr = PAGE_CACHE.lock().get_page(&private_key, false)?;
        unsafe { physical_memory::copy(shared_paddr, private_paddr, Arch::PAGE_SIZE) };
        mapping.private_pages.lock().insert(vaddr, ());
        self.vmap_write(vaddr, private_paddr);
        Ok(())
    }

    fn handle_file_private_partial(
        &self,
        vaddr: usize,
        mapping: &Mapping,
        inode_key: &INodeKey,
        offset: &usize,
        file_length: &usize,
    ) -> Result<bool, &'static str> {
        if vaddr - mapping.base + Arch::PAGE_SIZE <= *file_length {
            return Ok(false);
        }
        let private_key = PageKey::Anonymous {
            process_id: self.page_table,
            virtual_address: vaddr,
        };
        let private_paddr = PAGE_CACHE.lock().get_page(&private_key, false)?;
        if vaddr - mapping.base < *file_length {
            let shared_key = PageKey::File {
                inode_key: inode_key.clone(),
                offset: vaddr - mapping.base + offset,
            };
            let shared_paddr = PAGE_CACHE.lock().get_page(&shared_key, false)?;
            unsafe {
                let partial_file_length = file_length.rem_euclid(Arch::PAGE_SIZE);
                physical_memory::copy(shared_paddr, private_paddr, partial_file_length);
            }
        }
        mapping.private_pages.lock().insert(vaddr, ());
        self.vmap_write(vaddr, private_paddr);
        Ok(true)
    }

    fn already_has(
        &self,
        cause: &PageFaultConditions,
        vaddr: usize,
        mapping: &Mapping,
    ) -> Result<bool, &'static str> {
        if !mapping.private_pages.lock().contains_key(&vaddr) {
            return Ok(false);
        }
        let key = PageKey::Anonymous {
            process_id: self.page_table,
            virtual_address: vaddr,
        };
        if cause.contains(PageFaultConditions::WRITE) {
            let paddr = PAGE_CACHE.lock().get_page(&key, true)?;
            self.vmap_write(vaddr, paddr);
        } else {
            let paddr = PAGE_CACHE.lock().get_page(&key, false)?;
            self.vmap_read(vaddr, paddr);
        }
        Ok(true)
    }
}

impl VirtualMemory {
    // TODO: MJ said there might be a better way to changing mappings
    // in the future.
    fn invlpg(&self, vaddr: usize) {
        Arch::virtual_unmap(self.page_table as u64, vaddr as u64);
        Arch::shootdown_tlbs(self.page_table as u64, vaddr, Arch::PAGE_SIZE);
    }

    fn vmap_write(&self, vaddr: usize, paddr: usize) {
        Arch::virtual_map(
            self.page_table as u64,
            vaddr as u64,
            paddr as u64,
            PagingOptions::PRESENT | PagingOptions::CACHEABLE | PagingOptions::WRITABLE,
        );
    }

    fn vmap_read(&self, vaddr: usize, paddr: usize) {
        Arch::virtual_map(
            self.page_table as u64,
            vaddr as u64,
            paddr as u64,
            PagingOptions::PRESENT | PagingOptions::CACHEABLE,
        );
    }
}
