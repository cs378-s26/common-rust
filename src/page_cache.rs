// TODO: Integrate this with MJ's VMM (especially the shadowing)
// Currently, very divergent.

// TODO: Use blocking locks

// TODO: Consider locking stuff more effeciently

// TODO: Minimize unwraps

// TODO: Actual permissions for pages

// TODO: Proper error handling

// TODO: Create the reverse mapping

// TODO: Mark pages dirty

// TODO: change 'Arch::get_address_space' -> 'page_table'

use crate::arch::{Arch, ArchTrait};
use crate::free::FreeSet;
use crate::physical_memory::{self, HHDM_OFFSET, frame_alloc};
use crate::print::kprintln;
use crate::vfs::INodeKey;
use crate::virtual_memory::{PageFaultConditions, PagingOptions};
use alloc::boxed::Box;
use alloc::collections::btree_map::BTreeMap;
use alloc::sync::Arc;
use core::ptr;
use core::sync::atomic::{AtomicUsize, Ordering};
use intrusive_collections::{Bound::Included, RBTreeLink};
use intrusive_collections::{KeyAdapter, RBTree, intrusive_adapter};
use spin::{Mutex, Once};

unsafe fn copy_physical(src: usize, dst: usize, length: usize) {
    let hhdm = HHDM_OFFSET.get().unwrap();
    unsafe { ptr::copy_nonoverlapping((src + hhdm) as *const u8, (dst + hhdm) as *mut u8, length) };
}

struct Mapping {
    file: Option<(INodeKey, usize, Option<usize>)>,
    length: usize,
    shared: bool,
    base: usize,
    link: RBTreeLink,

    // Pages that need to be keepen track of manually (instead of
    // through page cache) TODO: We just need usize, not the extra
    // page key, since all pages are anonymous.
    private_pages: Mutex<BTreeMap<usize, PageKey>>,
    // TODO: Add a shared pages map, similar to anonymous, needed for eviction
}

intrusive_adapter!(MappingAdapter = Box<Mapping>: Mapping { link => RBTreeLink });
impl<'a> KeyAdapter<'a> for MappingAdapter {
    type Key = usize;
    fn get_key(&self, x: &'a Mapping) -> usize {
        x.base
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug)]
enum PageKey {
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
    references: BTreeMap<PageKey, ()>,
}

// TODO: Hahaha, ArcMutex for everything!!!

struct PageCache {
    map: BTreeMap<PageKey, Arc<Mutex<Page>>>,
}

impl PageCache {
    fn add_reference(&mut self, parent: PageKey, child: PageKey) {
        let mut page_cache = PAGE_CACHE.lock();
        let page = self.map.get_mut(&parent).unwrap();
        page.lock().references.insert(child.clone(), ());
        page_cache.map.insert(child.clone(), Arc::clone(page));
    }

    fn get_page(&mut self, key: PageKey, cow: bool) -> Option<usize> {
        match (self.map.get_mut(&key), &key) {
            (Some(page), key) => {
                let mut lock = page.lock();
                if !cow || lock.references.len() == 1 {
                    return Some(lock.address);
                }
                let refcount = Arc::clone(page);
                lock.references.remove(&key);
                let paddr = physical_memory::frame_alloc();
                unsafe { copy_physical(lock.address, paddr, Arch::PAGE_SIZE) };
                drop(lock);
                let mut references = BTreeMap::new();
                references.insert(key.clone(), ());
                self.map.insert(key.clone(), refcount);
                Some(paddr)
            }
            (None, PageKey::File { inode_key, offset }) => {
                let paddr = physical_memory::frame_alloc();
                if inode_key.filesystem_id != SHARED_ANONYMOUS_FILESYSTEM {
                    let inode = inode_key.get_inode().unwrap();
                    inode.read_page(paddr as *mut u8, *offset).unwrap();
                }
                self.map.insert(
                    key,
                    Arc::new(Mutex::new(Page {
                        address: paddr,
                        references: BTreeMap::new(),
                    })),
                );
                Some(paddr)
            }
            (None, key) => {
                let address = physical_memory::frame_alloc();
                let hhdm = HHDM_OFFSET.get().unwrap();
                unsafe { ptr::write_bytes((address + hhdm) as *mut u8, 0, Arch::PAGE_SIZE) };
                let mut references = BTreeMap::new();
                references.insert(key.clone(), ());
                self.map.insert(
                    key.clone(),
                    Arc::new(Mutex::new(Page {
                        address,
                        references,
                    })),
                );
                Some(address)
            }
        }
    }
}

const SHARED_ANONYMOUS_FILESYSTEM: usize = usize::MAX;
static SHARED_ANONYMOUS_COUNTER: AtomicUsize = AtomicUsize::new(0);
pub struct VirtualMemory {
    free_set: Mutex<FreeSet>,
    active_set: Mutex<RBTree<MappingAdapter>>,
    page_table: usize,
}

static PAGE_CACHE: Mutex<PageCache> = Mutex::new(PageCache {
    map: BTreeMap::new(),
});

impl Clone for VirtualMemory {
    // TODO: Make this less greedy with the locks
    fn clone(&self) -> Self {
        let page_table = Self::new_page_table();
        let mut page_cache = PAGE_CACHE.lock();
        let old_free_set = self.free_set.lock();
        let old_active_set = self.active_set.lock();
        let mut new_active_set = RBTree::new(MappingAdapter::new());
        for old_mapping in old_active_set.iter() {
            let mut new_private_pages = BTreeMap::new();
            for (old_address, old_page) in old_mapping.private_pages.lock().iter() {
                let key = PageKey::Anonymous {
                    process_id: page_table,
                    virtual_address: *old_address,
                };
                page_cache.add_reference(old_page.clone(), key.clone());
                new_private_pages.insert(*old_address, key);
                // TODO: Just change cr3 to flush entire TLB
                self.invlpg(*old_address);
            }
            let new_mapping = Mapping {
                file: old_mapping.file.clone(),
                link: RBTreeLink::new(),
                private_pages: Mutex::new(new_private_pages),
                ..*old_mapping
            };
            new_active_set.insert(Box::new(new_mapping));
        }
        let new_free_set = old_free_set.clone();
        Self {
            active_set: Mutex::new(new_active_set),
            free_set: Mutex::new(new_free_set),
            page_table,
        }
    }
}

static KERNEL_PAGE_TABLE: Once<usize> = Once::new();
impl VirtualMemory {
    fn consider_private_page(
        &self,
        cause: &PageFaultConditions,
        vaddr: usize,
        private_pages: &mut BTreeMap<usize, PageKey>,
    ) -> bool {
        if let Some(key) = private_pages.get(&vaddr) {
            if cause.contains(PageFaultConditions::PRESENT) {}
            if cause.contains(PageFaultConditions::WRITE) {
                let paddr = PAGE_CACHE.lock().get_page(key.clone(), true).unwrap();
                self.vmap_write(vaddr, paddr);
            } else {
                let paddr = PAGE_CACHE.lock().get_page(key.clone(), false).unwrap();
                self.vmap_read(vaddr, paddr);
            }
            true
        } else {
            false
        }
    }

    pub fn get_page_table(&self) -> usize {
        self.page_table
    }

    fn handle_anonymous_private(
        &self,
        cause: PageFaultConditions,
        vaddr: usize,
        mapping: &Mapping,
    ) {
        assert!(vaddr.is_multiple_of(Arch::PAGE_SIZE));
        if let Some(key) = mapping.private_pages.lock().get(&vaddr) {
            let paddr = PAGE_CACHE.lock().get_page(key.clone(), true).unwrap();
            self.vmap_write(vaddr, paddr);
            return;
        }
        let mut private_pages = mapping.private_pages.lock();
        if self.consider_private_page(&cause, vaddr, &mut private_pages) {
            return;
        }
        let key = PageKey::Anonymous {
            process_id: Arch::get_address_space() as usize,
            virtual_address: vaddr,
        };
        private_pages.insert(vaddr, key.clone());
        if cause.contains(PageFaultConditions::WRITE) {
            let paddr = PAGE_CACHE.lock().get_page(key, true).unwrap();
            self.vmap_write(vaddr, paddr);
        } else {
            let paddr = PAGE_CACHE.lock().get_page(key, false).unwrap();
            self.vmap_read(vaddr, paddr);
        }
    }

    // TODO: Make sure these pages are COW as well
    fn handle_file_private(
        &self,
        cause: PageFaultConditions,
        vaddr: usize,
        mapping: &Mapping,
        inode_key: &INodeKey,
        offset: &usize,
        file_length: &Option<usize>,
    ) {
        assert!(vaddr.is_multiple_of(Arch::PAGE_SIZE));
        let mut private_pages = mapping.private_pages.lock();
        if self.consider_private_page(&cause, vaddr, &mut private_pages) {
            return;
        }
        if let Some(file_length) = file_length
            && self.handle_partial_file_private(
                vaddr,
                mapping,
                inode_key,
                offset,
                file_length,
                &mut private_pages,
            )
        {
            return;
        }
        let shared_key = PageKey::File {
            inode_key: inode_key.clone(),
            offset: vaddr - mapping.base + offset,
        };
        let shared_paddr = PAGE_CACHE.lock().get_page(shared_key, false).unwrap();
        if !cause.contains(PageFaultConditions::WRITE) {
            self.vmap_read(vaddr, shared_paddr);
            return;
        }
        let private_key = PageKey::Anonymous {
            process_id: Arch::get_address_space() as usize,
            virtual_address: vaddr,
        };
        let private_paddr = PAGE_CACHE
            .lock()
            .get_page(private_key.clone(), false)
            .unwrap();
        unsafe { copy_physical(shared_paddr, private_paddr, Arch::PAGE_SIZE) };
        private_pages.insert(vaddr, private_key);
        self.vmap_write(vaddr, private_paddr);
    }

    fn handle_file_shared(
        &self,
        vaddr: usize,
        mapping: &Mapping,
        inode_key: &INodeKey,
        offset: &usize,
    ) {
        assert!(vaddr.is_multiple_of(Arch::PAGE_SIZE));
        let key = PageKey::File {
            inode_key: inode_key.clone(),
            offset: vaddr - mapping.base + offset,
        };
        let paddr = PAGE_CACHE.lock().get_page(key, false).unwrap();
        self.vmap_write(vaddr, paddr);
    }

    pub fn handle_page_fault(&self, cause: PageFaultConditions, address: usize) {
        // Aligned virtual address
        let vaddr = address & !(Arch::PAGE_SIZE - 1);
        let active_set = self.active_set.lock();
        let mapping = active_set.upper_bound(Included(&vaddr)).get().unwrap();
        if mapping.base + mapping.length <= vaddr {
            todo!()
        }
        if cause.contains(PageFaultConditions::PRESENT) {
            self.invlpg(vaddr);
        }
        if let Some((inode_key, offset, file_length)) = mapping.file.as_ref() {
            if mapping.shared {
                self.handle_file_shared(vaddr, mapping, inode_key, offset);
            } else {
                self.handle_file_private(cause, vaddr, mapping, inode_key, offset, file_length);
            }
        } else {
            assert!(!mapping.shared);
            self.handle_anonymous_private(cause, vaddr, mapping);
        }
    }

    // TODO: Make sure these pages are COW as well
    fn handle_partial_file_private(
        &self,
        vaddr: usize,
        mapping: &Mapping,
        inode_key: &INodeKey,
        offset: &usize,
        file_length: &usize,
        private_pages: &mut BTreeMap<usize, PageKey>,
    ) -> bool {
        if vaddr - mapping.base + Arch::PAGE_SIZE <= *file_length {
            return false;
        }
        let private_key = PageKey::Anonymous {
            process_id: Arch::get_address_space() as usize,
            virtual_address: vaddr,
        };
        let private_paddr = PAGE_CACHE
            .lock()
            .get_page(private_key.clone(), false)
            .unwrap();
        if vaddr - mapping.base < *file_length {
            let shared_key = PageKey::File {
                inode_key: inode_key.clone(),
                offset: vaddr - mapping.base + offset,
            };
            let shared_paddr = PAGE_CACHE.lock().get_page(shared_key, false).unwrap();
            unsafe {
                let partial_file_length = file_length.rem_euclid(Arch::PAGE_SIZE);
                copy_physical(shared_paddr, private_paddr, partial_file_length);
            }
        }
        private_pages.insert(vaddr, private_key);
        self.vmap_write(vaddr, private_paddr);
        true
    }

    pub fn init() {
        let kernel_page_table = KERNEL_PAGE_TABLE.call_once(|| Arch::get_address_space() as usize);
        kprintln!("Kernel page table: 0x{:x}", kernel_page_table);
        assert!(kernel_page_table.is_multiple_of(Arch::PAGE_SIZE));
    }

    fn invlpg(&self, vaddr: usize) {
        // TODO: MJ said there might be a better way in the future
        Arch::virtual_unmap(Arch::get_address_space(), vaddr as u64);
        Arch::shootdown_tlbs(Arch::get_address_space(), vaddr, Arch::PAGE_SIZE);
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
        if file.is_none() && shared == true {
            let inode_key = INodeKey::new(
                SHARED_ANONYMOUS_FILESYSTEM,
                SHARED_ANONYMOUS_COUNTER.fetch_add(1, Ordering::SeqCst),
            );
            file = Some((inode_key, 0, None));
        }
        let base: usize;
        let mut free_set = self.free_set.lock();
        if let Some(preferred_base) = preferred_base {
            match free_set.remove_range_by_base(preferred_base, length) {
                Ok(_) => base = preferred_base,
                Err(e) => return Err(e),
            }
        } else {
            match free_set.remove_range_by_length(length) {
                Ok(b) => base = b,
                Err(e) => return Err(e),
            }
        }
        let mut active_set = self.active_set.lock();
        active_set.insert(Box::new(Mapping {
            file,
            length,
            shared,
            base,
            link: RBTreeLink::new(),
            private_pages: Mutex::new(BTreeMap::new()),
        }));
        Ok(base)
    }

    pub fn new() -> Self {
        let mut set = FreeSet::new();
        // TODO: Maybe not hard code these values?
        set.add_range(0x10000, 0x8000_0000_0000_0000 - 0x10000)
            .unwrap();
        Self {
            free_set: Mutex::new(set),
            active_set: Mutex::new(RBTree::new(MappingAdapter::new())),
            page_table: Self::new_page_table(),
        }
    }

    pub fn new_page_table() -> usize {
        let page_table = frame_alloc();
        unsafe {
            copy_physical(
                *KERNEL_PAGE_TABLE.get().unwrap(),
                page_table,
                Arch::PAGE_SIZE,
            )
        };
        page_table
    }

    fn vmap_read(&self, vaddr: usize, paddr: usize) {
        Arch::virtual_map(
            Arch::get_address_space(),
            vaddr as u64,
            paddr as u64,
            PagingOptions::PRESENT | PagingOptions::CACHEABLE,
        );
    }

    fn vmap_write(&self, vaddr: usize, paddr: usize) {
        Arch::virtual_map(
            Arch::get_address_space(),
            vaddr as u64,
            paddr as u64,
            PagingOptions::PRESENT | PagingOptions::CACHEABLE | PagingOptions::WRITABLE,
        );
    }
    // TODO: munmap
}
