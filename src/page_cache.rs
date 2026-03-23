// TODO: Integrate this with MJ's VMM (especially the shadowing)
// Currently, very divergent.

// TODO: Use blocking locks

// TODO: Consider locking stuff more effeciently

// TODO: Minimize unwraps

// TODO: Actual permissions for pages

// TODO: Proper error handling

// TODO: Create the reverse mapping

// TODO: Mark pages dirty

use crate::arch::{Arch, ArchTrait};
use crate::free::FreeSet;
use crate::physical_memory::{self, HHDM_OFFSET, frame_alloc};
use crate::print::kprintln;
use crate::vfs::INodeKey;
use crate::virtual_memory::{PageFaultConditions, PagingOptions};
use alloc::boxed::Box;
use alloc::collections::btree_map::BTreeMap;
use core::ptr;
use intrusive_collections::{Bound::Included, RBTreeLink};
use intrusive_collections::{KeyAdapter, RBTree, intrusive_adapter};
use spin::{Mutex, Once};

struct Mapping {
    file: Option<(INodeKey, usize, Option<usize>)>,
    length: usize,
    shared: bool,
    base: usize,
    link: RBTreeLink,
    // Pages that need to be keepen track of manually (instead of
    // through page cache)
    exception_pages: Mutex<BTreeMap<usize, PageKey>>,
}

intrusive_adapter!(MappingAdapter = Box<Mapping>: Mapping { link => RBTreeLink });
impl<'a> KeyAdapter<'a> for MappingAdapter {
    type Key = usize;
    fn get_key(&self, x: &'a Mapping) -> usize {
        x.base
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone)]
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
}

struct PageCache {
    map: BTreeMap<PageKey, Page>,
}

impl PageCache {
    fn get_page(&mut self, key: PageKey) -> Option<usize> {
        match self.map.get(&key) {
            Some(p) => Some(p.address),
            None => match &key {
                PageKey::File { inode_key, offset } => {
                    let inode = inode_key.get_inode().unwrap();
                    let address = physical_memory::frame_alloc();
                    inode.read_page(address as *mut u8, *offset).unwrap();
                    self.map.insert(key, Page { address });
                    Some(address)
                }
                _ => {
                    let address = physical_memory::frame_alloc();
                    let hhdm = HHDM_OFFSET.get().unwrap();
                    unsafe { ptr::write_bytes((address + hhdm) as *mut u8, 0, Arch::PAGE_SIZE) };
                    self.map.insert(key, Page { address });
                    Some(address)
                }
            },
        }
    }
}

pub struct VirtualMemory {
    free_set: Mutex<FreeSet>,
    active_set: Mutex<RBTree<MappingAdapter>>,
    page_table: usize,
}

static PAGE_CACHE: Mutex<PageCache> = Mutex::new(PageCache {
    map: BTreeMap::new(),
});

static KERNEL_PAGE_TABLE: Once<usize> = Once::new();
impl VirtualMemory {
    pub fn init() {
        let kernel_page_table = KERNEL_PAGE_TABLE.call_once(|| Arch::get_address_space() as usize);
        kprintln!("Kernel page table: 0x{:x}", kernel_page_table);
        assert!(kernel_page_table.is_multiple_of(Arch::PAGE_SIZE));
    }

    pub fn get_page_table(&self) -> usize {
        self.page_table
    }

    // TODO: This needs to be changed when we have actual processes
    fn handle_anonymous(&self, address: usize) {
        assert!(address.is_multiple_of(Arch::PAGE_SIZE));
        let private_key = PageKey::Anonymous {
            process_id: Arch::get_address_space() as usize,
            virtual_address: address,
        };
        let private_address = PAGE_CACHE.lock().get_page(private_key.clone()).unwrap();
        Arch::virtual_map(
            Arch::get_address_space(),
            address as u64,
            private_address as u64,
            PagingOptions::PRESENT | PagingOptions::WRITABLE | PagingOptions::CACHEABLE,
        );
    }

    fn handle_file_private(
        &self,
        cause: PageFaultConditions,
        address: usize,
        mapping: &Mapping,
        inode_key: &INodeKey,
        offset: &usize,
        file_length: &Option<usize>,
    ) {
        assert!(address.is_multiple_of(Arch::PAGE_SIZE));
        let mut exception_pages = mapping.exception_pages.lock();
        if let Some(file_length) = file_length {
            if self.handle_partial_file_private(
                address,
                mapping,
                inode_key,
                offset,
                file_length,
                &mut exception_pages,
            ) {
                return;
            }
        }
        let shared_address = PAGE_CACHE
            .lock()
            .get_page(PageKey::File {
                inode_key: inode_key.clone(),
                offset: address - mapping.base + offset,
            })
            .unwrap();
        if !cause.contains(PageFaultConditions::WRITE) {
            Arch::virtual_map(
                Arch::get_address_space(),
                address as u64,
                shared_address as u64,
                PagingOptions::PRESENT | PagingOptions::CACHEABLE,
            );
            return;
        }
        let private_key = PageKey::Anonymous {
            process_id: Arch::get_address_space() as usize,
            virtual_address: address,
        };
        let private_address = PAGE_CACHE.lock().get_page(private_key.clone()).unwrap();
        unsafe {
            let hhdm = HHDM_OFFSET.get().unwrap();
            ptr::copy_nonoverlapping(
                (shared_address + hhdm) as *const u8,
                (private_address + hhdm) as *mut u8,
                Arch::PAGE_SIZE,
            );
        }
        exception_pages.insert(address, private_key);
        if cause.contains(PageFaultConditions::PRESENT) {
            // TODO: MJ said there might be a better way in the future
            Arch::virtual_unmap(Arch::get_address_space(), address as u64);
            Arch::shootdown_tlbs(Arch::get_address_space(), address, Arch::PAGE_SIZE);
        }
        Arch::virtual_map(
            Arch::get_address_space(),
            address as u64,
            private_address as u64,
            PagingOptions::PRESENT | PagingOptions::WRITABLE | PagingOptions::CACHEABLE,
        );
    }

    fn handle_file_shared(
        &self,
        address: usize,
        mapping: &Mapping,
        inode_key: &INodeKey,
        offset: &usize,
    ) {
        assert!(address.is_multiple_of(Arch::PAGE_SIZE));
        let physical_address = PAGE_CACHE
            .lock()
            .get_page(PageKey::File {
                inode_key: inode_key.clone(),
                offset: address - mapping.base + offset,
            })
            .unwrap();
        Arch::virtual_map(
            Arch::get_address_space(),
            address as u64,
            physical_address as u64,
            PagingOptions::PRESENT | PagingOptions::WRITABLE | PagingOptions::CACHEABLE,
        );
    }

    pub fn handle_page_fault(&self, cause: PageFaultConditions, address: usize) {
        // Align address
        let address = address & !(Arch::PAGE_SIZE - 1);
        let active_set = self.active_set.lock();
        let mapping = active_set.upper_bound(Included(&address)).get().unwrap();
        if mapping.base + mapping.length <= address {
            todo!()
        }
        if let Some((inode_key, offset, file_length)) = mapping.file.as_ref() {
            if mapping.shared {
                self.handle_file_shared(address, mapping, inode_key, offset);
            } else {
                self.handle_file_private(cause, address, mapping, inode_key, offset, file_length);
            }
        } else {
            self.handle_anonymous(address);
        }
    }

    fn handle_partial_file_private(
        &self,
        address: usize,
        mapping: &Mapping,
        inode_key: &INodeKey,
        offset: &usize,
        file_length: &usize,
        exception_pages: &mut BTreeMap<usize, PageKey>,
    ) -> bool {
        if address - mapping.base + Arch::PAGE_SIZE <= *file_length {
            return false;
        }
        let private_key = PageKey::Anonymous {
            process_id: Arch::get_address_space() as usize,
            virtual_address: address,
        };
        let private_address = PAGE_CACHE.lock().get_page(private_key.clone()).unwrap();
        if address - mapping.base < *file_length {
            let shared_address = PAGE_CACHE
                .lock()
                .get_page(PageKey::File {
                    inode_key: inode_key.clone(),
                    offset: address - mapping.base + offset,
                })
                .unwrap();
            unsafe {
                let hhdm = HHDM_OFFSET.get().unwrap();
                let partial_file_length = file_length.rem_euclid(Arch::PAGE_SIZE);
                ptr::copy_nonoverlapping(
                    (shared_address + hhdm) as *const u8,
                    (private_address + hhdm) as *mut u8,
                    partial_file_length,
                );
            }
        }
        exception_pages.insert(address, private_key);
        Arch::virtual_map(
            Arch::get_address_space(),
            address as u64,
            private_address as u64,
            PagingOptions::PRESENT | PagingOptions::WRITABLE | PagingOptions::CACHEABLE,
        );
        true
    }

    pub fn mmap(
        &self,
        file: Option<(INodeKey, usize, Option<usize>)>,
        length: usize,
        shared: bool,
        preferred_base: Option<usize>,
    ) -> Result<usize, &'static str> {
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
            exception_pages: Mutex::new(BTreeMap::new()),
        }));
        Ok(base)
    }

    pub fn new() -> Self {
        let mut set = FreeSet::new();
        set.add_range(0x10000, 0x8000_0000_0000_0000 - 0x10000)
            .unwrap();
        let page_table = frame_alloc();
        unsafe {
            let hhdm = HHDM_OFFSET.get().unwrap();
            ptr::copy_nonoverlapping(
                (*KERNEL_PAGE_TABLE.get().unwrap() + hhdm) as *const u8,
                (page_table + hhdm) as *mut u8,
                Arch::PAGE_SIZE,
            );
        }
        Self {
            free_set: Mutex::new(set),
            active_set: Mutex::new(RBTree::new(MappingAdapter::new())),
            page_table,
        }
    }
    // TODO: munmap
}
