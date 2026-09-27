// heap
// TODO: use virtual memory herez
pub static mut THE_HEAP: [u8; 256 * 1024 * 1024] = [0; _];

use alloc::string::{String, ToString};
use core::{array, cell::Cell, iter, mem, ptr};

use bitflags::bitflags;
use intrusive_collections::{LinkedList, LinkedListLink, UnsafeRef, intrusive_adapter};
use limine::{
    memory_map::{Entry, EntryType},
    request::{HhdmRequest, MemoryMapRequest},
};
use spin::Once;

use crate::{
    arch::{Arch, ArchTrait},
    kprintln,
    memory::virtual_memory::{PagingOptions, VirtualMemoryAllocation},
    sync::{IntMutex, MutexLike},
};

// the below Limine-related code is partially from ChatGPT

#[unsafe(link_section = ".limine_requests")]
static MEMMAP_REQUEST: MemoryMapRequest = MemoryMapRequest::new();

#[unsafe(link_section = ".limine_requests")]
pub static HHDM_REQUEST: HhdmRequest = HhdmRequest::new();

pub static REGIONS: Once<&[&Entry]> = Once::new();
pub static HHDM_OFFSET: Once<usize> = Once::new();

/// Largest buddy block is 2^MAX_ORDER frames.
const MAX_ORDER: usize = 10;

pub trait PageFrameAllocator {
    fn alloc_frame(&mut self) -> Option<usize>;
    fn dealloc_frame(&mut self, frame: usize);
}

fn display_entry_type(et: EntryType) -> String {
    match et {
        EntryType::USABLE => "Usable",
        EntryType::RESERVED => "Reserved permanently",
        EntryType::ACPI_RECLAIMABLE => "Reclaimable from ACPI",
        EntryType::ACPI_NVS => "Reserved for ACPI",
        EntryType::BAD_MEMORY => "Unusable hardware",
        EntryType::BOOTLOADER_RECLAIMABLE => "Reclaimable from Limine",
        EntryType::EXECUTABLE_AND_MODULES => "Reserved for kernel code",
        EntryType::FRAMEBUFFER => "Reserved for frame buffer",
        _ => panic!("Unexpected Limine memory map entry type"),
    }
    .to_string()
}

/// Usable regions as `[start, end)` frame number ranges.
fn usable_regions() -> impl Iterator<Item = (usize, usize)> {
    REGIONS
        .get()
        .unwrap()
        .iter()
        .filter(|entry| entry.entry_type == EntryType::USABLE)
        .map(|entry| {
            let base = entry.base as usize;
            (
                base / Arch::PAGE_SIZE,
                (base + entry.length as usize) / Arch::PAGE_SIZE,
            )
        })
}

/// Bump allocator over the usable regions, used until the buddy allocator takes over.
pub struct EarlyPmm {
    region: usize,
    next: usize,
    end: usize,
}

impl EarlyPmm {
    pub fn init() -> Self {
        HHDM_OFFSET.call_once(|| HHDM_REQUEST.get_response().unwrap().offset() as usize);
        REGIONS.call_once(|| {
            let entries = MEMMAP_REQUEST.get_response().unwrap().entries();
            kprintln!("\nLimine Memory Map:");
            for entry in entries {
                kprintln!(
                    "{:016x}-{:016x} ({})",
                    entry.base,
                    entry.base + entry.length,
                    display_entry_type(entry.entry_type)
                );
            }
            entries
        });
        let (next, end) = usable_regions()
            .next()
            .expect("No usable memory regions found");
        Self {
            region: 0,
            next,
            end,
        }
    }

    fn unused(&self) -> impl Iterator<Item = (usize, usize)> {
        iter::once((self.next, self.end)).chain(usable_regions().skip(self.region + 1))
    }
}

impl PageFrameAllocator for EarlyPmm {
    fn alloc_frame(&mut self) -> Option<usize> {
        while self.next == self.end {
            self.region += 1;
            (self.next, self.end) = usable_regions().nth(self.region)?;
        }
        let frame = self.next;
        self.next += 1;
        Some(frame * Arch::PAGE_SIZE)
    }

    fn dealloc_frame(&mut self, _frame: usize) {}
}

bitflags! {
    #[derive(Clone, Copy)]
    struct PageFlags: u8 {
        const FREE = 1 << 0;
    }
}

pub struct Page {
    link: LinkedListLink,
    flags: Cell<PageFlags>,
    order: Cell<u8>,
}

intrusive_adapter!(FreeAdapter = UnsafeRef<Page>: Page { link => LinkedListLink });

impl Page {
    const fn new() -> Self {
        Self {
            link: LinkedListLink::new(),
            flags: Cell::new(PageFlags::empty()),
            order: Cell::new(0),
        }
    }

    fn is_free_at(&self, order: usize) -> bool {
        self.flags.get().contains(PageFlags::FREE) && self.order.get() as usize == order
    }
}

/// Virtually contiguous `Page` per frame, indexed by pfn. Only the parts covering usable memory
/// (rounded out to buddy blocks) are backed.
pub struct PageArray {
    base: usize,
    pages: *mut Page,
}

impl PageArray {
    pub fn map(early: &mut EarlyPmm) -> Self {
        let block = 1 << MAX_ORDER;
        let spans = || {
            usable_regions().map(|(start, end)| (start & !(block - 1), end.next_multiple_of(block)))
        };
        let base = spans().map(|(start, _)| start).min().unwrap();
        let end = spans().map(|(_, end)| end).max().unwrap();

        let space = Arch::get_kernel_address_space();
        let options = PagingOptions::PRESENT | PagingOptions::WRITABLE | PagingOptions::CACHEABLE;
        let bytes = ((end - base) * size_of::<Page>()).next_multiple_of(Arch::PAGE_SIZE);
        let vma = VirtualMemoryAllocation::new(space, None, bytes, None, options, false)
            .expect("failed to reserve virtual memory for the page array");
        let array = Self {
            base,
            pages: vma.base as *mut Page,
        };
        mem::forget(vma);

        let mut mapped = 0;
        for (start, end) in spans() {
            let lo = (array.slot(start) as usize & !(Arch::PAGE_SIZE - 1)).max(mapped);
            let hi = (array.slot(end) as usize).next_multiple_of(Arch::PAGE_SIZE);
            for vaddr in (lo..hi).step_by(Arch::PAGE_SIZE) {
                let frame = early
                    .alloc_frame()
                    .expect("out of memory mapping the page array");
                Arch::virtual_map_with(space, vaddr as u64, frame as u64, options, early);
            }
            mapped = mapped.max(hi);
            for pfn in start..end {
                unsafe { array.slot(pfn).write(Page::new()) };
            }
        }
        kprintln!(
            "page array at {:p} covering frames {:x}-{:x}",
            array.pages,
            base,
            end
        );
        array
    }

    fn slot(&self, pfn: usize) -> *mut Page {
        self.pages.wrapping_add(pfn - self.base)
    }

    fn get(&self, pfn: usize) -> &'static Page {
        unsafe { &*self.slot(pfn) }
    }

    fn pfn_of(&self, page: &Page) -> usize {
        self.base + unsafe { ptr::from_ref(page).offset_from_unsigned(self.pages) }
    }
}

pub struct BuddyPmm {
    pages: PageArray,
    free: [LinkedList<FreeAdapter>; MAX_ORDER + 1],
}

// only reachable through PMM's lock
unsafe impl Send for BuddyPmm {}

impl BuddyPmm {
    fn migrate(early: EarlyPmm, pages: PageArray) -> Self {
        let mut pmm = Self {
            pages,
            free: array::from_fn(|_| LinkedList::new(FreeAdapter::new())),
        };
        for (start, end) in early.unused() {
            pmm.free_range(start, end);
        }
        pmm
    }

    fn push(&mut self, pfn: usize, order: usize) {
        let page = self.pages.get(pfn);
        page.flags.set(PageFlags::FREE);
        page.order.set(order as u8);
        self.free[order].push_front(unsafe { UnsafeRef::from_raw(page) });
    }

    fn unlink(&mut self, page: &Page) {
        page.flags.set(PageFlags::empty());
        unsafe {
            self.free[page.order.get() as usize]
                .cursor_mut_from_ptr(page)
                .remove()
        };
    }

    fn alloc_order(&mut self, order: usize) -> Option<usize> {
        let found = (order..=MAX_ORDER).find(|&o| !self.free[o].is_empty())?;
        let page = self.free[found].pop_front()?;
        page.flags.set(PageFlags::empty());
        let pfn = self.pages.pfn_of(&page);
        for split in (order..found).rev() {
            self.push(pfn + (1 << split), split);
        }
        Some(pfn)
    }

    fn free_order(&mut self, mut pfn: usize, mut order: usize) {
        assert!(
            !self.pages.get(pfn).flags.get().contains(PageFlags::FREE),
            "double free of frame {:x}",
            pfn * Arch::PAGE_SIZE
        );
        while order < MAX_ORDER {
            let buddy = self.pages.get(pfn ^ (1 << order));
            if !buddy.is_free_at(order) {
                break;
            }
            self.unlink(buddy);
            pfn &= !((2 << order) - 1);
            order += 1;
        }
        self.push(pfn, order);
    }

    fn free_range(&mut self, mut start: usize, end: usize) {
        while start < end {
            let order = (start.trailing_zeros() as usize)
                .min((end - start).ilog2() as usize)
                .min(MAX_ORDER);
            self.free_order(start, order);
            start += 1 << order;
        }
    }

    fn alloc_contiguous(&mut self, frames: usize) -> Option<usize> {
        let order = frames.next_power_of_two().trailing_zeros() as usize;
        let pfn = self.alloc_order(order)?;
        self.free_range(pfn + frames, pfn + (1 << order));
        Some(pfn)
    }
}

static PMM: Once<IntMutex<BuddyPmm>> = Once::new();

pub fn init(early: EarlyPmm, pages: PageArray) {
    PMM.call_once(|| IntMutex::new(BuddyPmm::migrate(early, pages)));
}

fn pmm() -> &'static IntMutex<BuddyPmm> {
    PMM.get()
        .expect("physical memory allocator used before init_mm")
}

pub struct GlobalPmm;

impl PageFrameAllocator for GlobalPmm {
    fn alloc_frame(&mut self) -> Option<usize> {
        pmm().lock().alloc_order(0).map(|pfn| pfn * Arch::PAGE_SIZE)
    }

    fn dealloc_frame(&mut self, frame: usize) {
        assert!(frame.is_multiple_of(Arch::PAGE_SIZE));
        pmm().lock().free_order(frame / Arch::PAGE_SIZE, 0);
    }
}

pub fn frame_alloc() -> usize {
    GlobalPmm.alloc_frame().expect("out of physical memory")
}

/// Allocates `frames` physically contiguous frames, each freed individually with `frame_dealloc`.
pub fn alloc_frames(frames: usize) -> usize {
    assert!(frames > 0);
    pmm()
        .lock()
        .alloc_contiguous(frames)
        .map(|pfn| pfn * Arch::PAGE_SIZE)
        .unwrap_or_else(|| panic!("failed to allocate {frames} contiguous frames"))
}

pub fn frame_dealloc(frame: usize) {
    GlobalPmm.dealloc_frame(frame);
}

/// # Safety
///
/// This function treats src and dst as raw pointers to physical (not
/// virtual) memory. It is expected that src and dst are NOT already
/// adjusted by the HHDM offset, as this function does that for you.
pub unsafe fn copy(src: usize, dst: usize, length: usize) {
    let hhdm = HHDM_OFFSET.get().unwrap();
    unsafe { ptr::copy_nonoverlapping((src + hhdm) as *const u8, (dst + hhdm) as *mut u8, length) };
}
