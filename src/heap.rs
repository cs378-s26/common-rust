use core::{
    alloc::{GlobalAlloc, Layout},
    ptr::{null_mut, slice_from_raw_parts_mut},
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::sync::MutexLike;

use crate::{
    arch::PAGE_SIZE,
    mp::{CORE_ID, MP_STAGE, MPStage},
    print::kprintln,
    sync::IntMutex,
};

use alloc::boxed::Box;
use limine::memory_map::{Entry, EntryType};
const MAX_CPUS: usize = 256;
// used for debugging 
const ENABLE_PER_CPU_CACHE: bool = true;

const NUM_CLASSES: usize = 8;
const CLASS_SIZES: [usize; NUM_CLASSES] = [16, 32, 64, 128, 256, 512, 1024, 2048];
const MAX_SLAB_SIZE: usize = CLASS_SIZES[NUM_CLASSES - 1];
const SLAB_PAGE_MAGIC: u32 = 0x12AB_CDEF;
// good info on slab allocator / buddy allocator
// https://hammertux.github.io/slab-allocator
// https://www.youtube.com/watch?v=OvZyZaMC0f0 ("Buddy and Slab Allocators" Operating Systems Course at University of Toronto)
/// MAX_ORDER is in pages (order 0 = 1 page, order 1 = 2 page, etc...)
/// MAX_ORDER=18 => max block is 2^18 pages = 1GiB at 4KiB pages.
const MAX_ORDER: usize = 18;
const BUDDY_NONE: u64 = u64::MAX;
const BUDDY_FREE_MAGIC: u32 = 0xBDDF_FACE;

const MAX_REGIONS: usize = 256;

static PER_CPU_CACHE: [[AtomicUsize; NUM_CLASSES]; MAX_CPUS] =
    [const { [const { AtomicUsize::new(0) }; NUM_CLASSES] }; MAX_CPUS];

#[repr(C)]
struct SlabPageHeader {
    magic: u32,
    class_index: u16,
    in_use: u16,
    capacity: u16,
    _reserved: u16,
    free_list: *mut u8,
    next: *mut SlabPageHeader,
}

#[repr(C)]
struct FreeBlockNode {
    magic: u32, // BUDDY_FREE_MAGIC when on a free list
    order: u32, // order this block is free at
    next: u64,  // phys address of next block (or BUDDY_NONE)
    prev: u64,  // phys address of prev block (or BUDDY_NONE)
}

#[derive(Clone, Copy)]
struct PhysRegion {
    base: u64, // inclusive, page-aligned
    end: u64,  // exclusive, page-aligned
}

const EMPTY_REGION: PhysRegion = PhysRegion { base: 0, end: 0 };

struct BuddyPageAllocator {
    free_heads: [u64; MAX_ORDER + 1], // per-order list head (phys), or BUDDY_NONE
    regions: [PhysRegion; MAX_REGIONS],
    region_count: usize,
}

impl BuddyPageAllocator {
    const fn new() -> Self {
        Self {
            free_heads: [BUDDY_NONE; MAX_ORDER + 1],
            regions: [EMPTY_REGION; MAX_REGIONS],
            region_count: 0,
        }
    }

    fn reset(&mut self) {
        self.free_heads = [BUDDY_NONE; MAX_ORDER + 1];
        self.regions = [EMPTY_REGION; MAX_REGIONS];
        self.region_count = 0;
    }

    fn add_region(&mut self, base: u64, end: u64) {
        if self.region_count >= MAX_REGIONS {
            kprintln!(
                "mem::buddy: too many regions (MAX_REGIONS={}), dropping region [{:#x}, {:#x})",
                MAX_REGIONS,
                base,
                end
            );
            return;
        }
        self.regions[self.region_count] = PhysRegion { base, end };
        self.region_count += 1;
    }

    #[inline(always)]
    fn block_size_bytes(order: usize) -> u64 {
        (PAGE_SIZE as u64) << order
    }

    /// For a requested number of pages, return the buddy order to allocate,
    /// rounding up to power-of-two pages.
    #[inline(always)]
    fn order_for_pages(pages: usize) -> Option<usize> {
        if pages == 0 {
            return None;
        }
        let p2 = pages.next_power_of_two();
        let order = p2.trailing_zeros() as usize;
        if order > MAX_ORDER { None } else { Some(order) }
    }

    fn contains_in_same_region(&self, addr: u64, size: u64) -> bool {
        let end = addr.wrapping_add(size);
        for i in 0..self.region_count {
            let r = self.regions[i];
            if addr >= r.base && end <= r.end {
                return true;
            }
        }
        false
    }

    #[inline(always)]
    fn push_free(&mut self, hhdm_offset: u64, order: usize, phys: u64) {
        let old_head = self.free_heads[order];
        let virt = (phys + hhdm_offset) as *mut FreeBlockNode;
        unsafe {
            (*virt).magic = BUDDY_FREE_MAGIC;
            (*virt).order = order as u32;
            (*virt).next = old_head;
            (*virt).prev = BUDDY_NONE;
        }
        if old_head != BUDDY_NONE {
            let old_virt = (old_head + hhdm_offset) as *mut FreeBlockNode;
            unsafe { (*old_virt).prev = phys };
        }
        self.free_heads[order] = phys;
    }

    #[inline(always)]
    fn pop_free(&mut self, hhdm_offset: u64, order: usize) -> Option<u64> {
        let head = self.free_heads[order];
        if head == BUDDY_NONE {
            return None;
        }
        let virt = (head + hhdm_offset) as *mut FreeBlockNode;
        let next = unsafe {
            (*virt).magic = 0;
            (*virt).next
        };
        if next != BUDDY_NONE {
            let next_virt = (next + hhdm_offset) as *mut FreeBlockNode;
            unsafe { (*next_virt).prev = BUDDY_NONE };
        }
        self.free_heads[order] = next;
        Some(head)
    }

    /// O(1) buddy removal. check if `target` is free at `order` via its
    /// magic/order fields, then unlink using prev/next pointers.
    fn try_remove_buddy(&mut self, hhdm_offset: u64, order: usize, target: u64) -> bool {
        let virt = (target + hhdm_offset) as *mut FreeBlockNode;
        unsafe {
            if (*virt).magic != BUDDY_FREE_MAGIC || (*virt).order != order as u32 {
                return false;
            }

            let prev = (*virt).prev;
            let next = (*virt).next;

            if prev == BUDDY_NONE {
                self.free_heads[order] = next;
            } else {
                let prev_node = (prev + hhdm_offset) as *mut FreeBlockNode;
                (*prev_node).next = next;
            }

            if next != BUDDY_NONE {
                let next_node = (next + hhdm_offset) as *mut FreeBlockNode;
                (*next_node).prev = prev;
            }

            (*virt).magic = 0;
        }
        true
    }

    /// Add a usable range into the buddy allocator by carving it into aligned power-of-two blocks.
    fn add_range(&mut self, hhdm_offset: u64, mut base: u64, mut pages: usize) {
        if pages == 0 {
            return;
        }

        let end = base + (pages as u64) * (PAGE_SIZE as u64);
        self.add_region(base, end);

        while pages > 0 {
            // Pick largest order such that:
            //  - block_pages <= pages
            //  - base is aligned to block size in bytes
            let mut order = MAX_ORDER;
            loop {
                let block_pages = 1usize << order;
                let block_bytes = (block_pages as u64) * (PAGE_SIZE as u64);

                if block_pages <= pages && (base % block_bytes) == 0 {
                    break;
                }

                if order == 0 {
                    break;
                }
                order -= 1;
            }

            let used_pages = 1usize << order;
            self.push_free(hhdm_offset, order, base);

            base += (used_pages as u64) * (PAGE_SIZE as u64);
            pages -= used_pages;
        }
    }

    fn alloc_pages(&mut self, hhdm_offset: u64, order: usize) -> Option<u64> {
        if order > MAX_ORDER {
            return None;
        }

        // Find first non-empty list at >= order.
        let mut cur_order = order;
        while cur_order <= MAX_ORDER {
            if self.free_heads[cur_order] != BUDDY_NONE {
                break;
            }
            cur_order += 1;
        }
        if cur_order > MAX_ORDER {
            return None;
        }

        let mut block = self.pop_free(hhdm_offset, cur_order)?;

        // Split down to requested order.
        while cur_order > order {
            cur_order -= 1;
            let half_size = Self::block_size_bytes(cur_order);
            let buddy = block + half_size;
            self.push_free(hhdm_offset, cur_order, buddy);
            // keep `block` as the first half
        }

        Some(block)
    }

    fn free_pages(&mut self, hhdm_offset: u64, mut base: u64, mut order: usize) {
        if order > MAX_ORDER {
            return;
        }

        loop {
            let size = Self::block_size_bytes(order);
            let buddy = base ^ size;

            // Only attempt to coalesce if the combined block stays within a single known region.
            let combined_base = base.min(buddy);
            if !self.contains_in_same_region(combined_base, size * 2) {
                self.push_free(hhdm_offset, order, base);
                return;
            }

            // If buddy is free at this order, remove it and coalesce.
            if self.try_remove_buddy(hhdm_offset, order, buddy) {
                base = combined_base;
                order += 1;
                if order > MAX_ORDER {
                    self.push_free(hhdm_offset, MAX_ORDER, base);
                    return;
                }
                continue;
            }

            // Buddy not free, can't coalesce.
            self.push_free(hhdm_offset, order, base);
            return;
        }
    }
}

struct HeapAllocator {
    hhdm_offset: u64,

    partial_slabs: [*mut SlabPageHeader; NUM_CLASSES],
    full_slabs: [*mut SlabPageHeader; NUM_CLASSES],

    buddy: BuddyPageAllocator,
}

unsafe impl Send for HeapAllocator {}

impl HeapAllocator {
    const fn new() -> Self {
        Self {
            hhdm_offset: 0,
            partial_slabs: [null_mut(); NUM_CLASSES],
            full_slabs: [null_mut(); NUM_CLASSES],
            buddy: BuddyPageAllocator::new(),
        }
    }

    fn init_from_memory_map(&mut self, hhdm_offset: u64, entries: &[&Entry]) {
        self.hhdm_offset = hhdm_offset;
        self.buddy.reset();

        let mut pages_total = 0usize;

        for entry in entries {
            if entry.entry_type != EntryType::USABLE {
                continue;
            }

            let start = align_up(entry.base as usize, PAGE_SIZE);
            let end = align_down((entry.base + entry.length) as usize, PAGE_SIZE);
            if end <= start {
                continue;
            }

            let page_count = (end - start) / PAGE_SIZE;
            if page_count == 0 {
                continue;
            }

            self.buddy
                .add_range(self.hhdm_offset, start as u64, page_count);
            pages_total += page_count;
        }

        kprintln!(
            "init_malloc():  allocator initialized, {} pages available",
            pages_total
        );
    }

    fn allocate(&mut self, layout: Layout) -> *mut u8 {
        if let Some(class) = class_index_for_layout(layout) {
            return self.allocate_small(class);
        }

        self.allocate_large(layout)
    }

    fn free(&mut self, ptr: *mut u8, layout: Layout) {
        if ptr.is_null() {
            return;
        }

        if class_index_for_layout(layout).is_some() {
            self.free_small(ptr);
            return;
        }

        self.free_large(ptr, layout);
    }

    fn allocate_small(&mut self, class: usize) -> *mut u8 {
        loop {
            let page = self.partial_slabs[class];
            if !page.is_null() {
                unsafe {
                    let object = (*page).free_list;
                    (*page).free_list = *(object as *mut *mut u8);
                    (*page).in_use += 1;

                    if (*page).free_list.is_null() {
                        self.partial_slabs[class] = (*page).next;
                        (*page).next = self.full_slabs[class];
                        self.full_slabs[class] = page;
                    }

                    return object;
                }
            }

            if !self.add_slab_page(class) {
                return null_mut();
            }
        }
    }

    fn free_small(&mut self, ptr: *mut u8) {
        let page_base = align_down(ptr as usize, PAGE_SIZE);
        let header = page_base as *mut SlabPageHeader;

        unsafe {
            if (*header).magic != SLAB_PAGE_MAGIC {
                kprintln!(
                    "mem::free_small(): invalid slab header for ptr={:p}",
                    ptr,
                );
                return;
            }
            let class = (*header).class_index as usize;

            let was_full = (*header).free_list.is_null();

            *(ptr as *mut *mut u8) = (*header).free_list;
            (*header).free_list = ptr;
            (*header).in_use -= 1;

            if (*header).in_use == 0 {
                if was_full {
                    Self::unlink_slab_page(&mut self.full_slabs[class], header);
                } else {
                    Self::unlink_slab_page(&mut self.partial_slabs[class], header);
                }

                let phys = self.virt_to_phys(page_base as u64);
                self.buddy.free_pages(self.hhdm_offset, phys, 0);
            } else if was_full {
                Self::unlink_slab_page(&mut self.full_slabs[class], header);
                (*header).next = self.partial_slabs[class];
                self.partial_slabs[class] = header;
            }
        }
    }

    fn allocate_large(&mut self, layout: Layout) -> *mut u8 {
        if layout.align() > PAGE_SIZE {
            return null_mut();
        }

        let pages_needed = page_count_for(layout.size());
        let Some(order) = BuddyPageAllocator::order_for_pages(pages_needed) else {
            return null_mut();
        };

        let phys = match self.buddy.alloc_pages(self.hhdm_offset, order) {
            Some(p) => p,
            None => return null_mut(),
        };

        self.phys_to_virt(phys) as *mut u8
    }

    fn free_large(&mut self, ptr: *mut u8, layout: Layout) {
        let phys = self.virt_to_phys(ptr as u64);
        let pages_needed = page_count_for(layout.size());
        let Some(order) = BuddyPageAllocator::order_for_pages(pages_needed) else {
            return;
        };

        self.buddy.free_pages(self.hhdm_offset, phys, order);
    }

    fn add_slab_page(&mut self, class: usize) -> bool {
        let phys = match self.buddy.alloc_pages(self.hhdm_offset, 0) {
            Some(p) => p,
            None => return false,
        };

        let virt = self.phys_to_virt(phys) as usize;
        let header = virt as *mut SlabPageHeader;
        let object_size = CLASS_SIZES[class];

        unsafe {
            let start = align_up(
                virt + core::mem::size_of::<SlabPageHeader>(),
                object_size.max(8),
            );
            if start >= virt + PAGE_SIZE {
                self.buddy.free_pages(self.hhdm_offset, phys, 0);
                return false;
            }

            let capacity = (virt + PAGE_SIZE - start) / object_size;
            if capacity == 0 {
                self.buddy.free_pages(self.hhdm_offset, phys, 0);
                return false;
            }

            let mut prev: *mut u8 = null_mut();
            for i in (0..capacity).rev() {
                let slot = (start + (i * object_size)) as *mut u8;
                *(slot as *mut *mut u8) = prev;
                prev = slot;
            }

            (*header).magic = SLAB_PAGE_MAGIC;
            (*header).class_index = class as u16;
            (*header).in_use = 0;
            (*header).capacity = capacity as u16;
            (*header)._reserved = 0;
            (*header).free_list = prev;
            (*header).next = self.partial_slabs[class];
            self.partial_slabs[class] = header;
        }

        true
    }

    fn unlink_slab_page(head: &mut *mut SlabPageHeader, victim: *mut SlabPageHeader) {
        let mut cur = *head;
        let mut prev: *mut SlabPageHeader = null_mut();

        unsafe {
            while !cur.is_null() {
                if cur == victim {
                    if prev.is_null() {
                        *head = (*cur).next;
                    } else {
                        (*prev).next = (*cur).next;
                    }
                    return;
                }

                prev = cur;
                cur = (*cur).next;
            }
        }
    }

    #[inline(always)]
    fn phys_to_virt(&self, phys: u64) -> u64 {
        phys + self.hhdm_offset
    }

    #[inline(always)]
    fn virt_to_phys(&self, virt: u64) -> u64 {
        virt - self.hhdm_offset
    }
}

struct GlobalAllocImpl {
    heap: IntMutex<HeapAllocator>,
}

fn try_current_cpu() -> Option<usize> {
    if MP_STAGE.load(Ordering::Relaxed) != MPStage::MPPreempt {
        return None;
    }

    let cpu = CORE_ID.get().0;
    if cpu < MAX_CPUS { Some(cpu) } else { None }
}

fn try_take_from_cpu_cache(class: usize) -> *mut u8 {
    if !ENABLE_PER_CPU_CACHE {
        return null_mut();
    }

    let Some(cpu) = try_current_cpu() else {
        return null_mut();
    };

    let value = PER_CPU_CACHE[cpu][class].swap(0, Ordering::AcqRel);
    value as *mut u8
}

fn try_put_into_cpu_cache(class: usize, ptr: *mut u8) -> Option<*mut u8> {
    if !ENABLE_PER_CPU_CACHE {
        return None;
    }

    let Some(cpu) = try_current_cpu() else {
        return None;
    };

    let slot = &PER_CPU_CACHE[cpu][class];
    let replaced = slot.swap(ptr as usize, Ordering::AcqRel);
    Some(replaced as *mut u8)
}

unsafe impl GlobalAlloc for GlobalAllocImpl {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        let layout = normalize_layout(layout);

        if let Some(class) = class_index_for_layout(layout) {
            let cached = try_take_from_cpu_cache(class);
            if !cached.is_null() {
                return cached;
            }
        }

        self.heap.lock().allocate(layout)
    }

    unsafe fn dealloc(&self, mut ptr: *mut u8, layout: core::alloc::Layout) {
        let layout = normalize_layout(layout);

        if let Some(class) = class_index_for_layout(layout)
            && let Some(replaced) = try_put_into_cpu_cache(class, ptr)
        {
            if replaced.is_null() {
                return;
            }
            ptr = replaced;
        }

        self.heap.lock().free(ptr, layout);
    }

    unsafe fn realloc(
        &self,
        ptr: *mut u8,
        old_layout: core::alloc::Layout,
        new_size: usize,
    ) -> *mut u8 {
        if ptr.is_null() {
            let new_layout = Layout::from_size_align(new_size.max(1), old_layout.align())
                .expect("invalid layout in realloc");
            return unsafe { self.alloc(new_layout) };
        }

        if new_size == 0 {
            unsafe { self.dealloc(ptr, old_layout) };
            return null_mut();
        }

        let old_layout = normalize_layout(old_layout);
        let new_layout =
            Layout::from_size_align(new_size, old_layout.align()).expect("invalid realloc layout");

        // In-place resize is only valid for slab allocations that stay in the
        // same size class. For large/page allocations, we must reallocate even
        // if both layouts map to `None`.
        if let (Some(old_class), Some(new_class)) = (
            class_index_for_layout(old_layout),
            class_index_for_layout(new_layout),
        ) && old_class == new_class
        {
            return ptr;
        }

        let new_ptr = unsafe { self.alloc(new_layout) };
        if new_ptr.is_null() {
            return null_mut();
        }

        unsafe {
            new_ptr.copy_from_nonoverlapping(ptr, old_layout.size().min(new_size));
            self.dealloc(ptr, old_layout);
        }

        new_ptr
    }
}

#[global_allocator]
static GLOBAL_ALLOC: GlobalAllocImpl = GlobalAllocImpl {
    heap: IntMutex::new(HeapAllocator::new()),
};

pub fn init_malloc(hhdm_offset: u64, entries: &[&Entry]) {
    kprintln!("init_malloc(): initializing  allocator");

    GLOBAL_ALLOC
        .heap
        .lock()
        .init_from_memory_map(hhdm_offset, entries);
}

pub fn aligned_slice(size: usize, align: usize) -> Box<[u8]> {
    let layout = Layout::from_size_align(size, align).expect("invalid aligned_slice layout");
    let ptr = unsafe { alloc::alloc::alloc(layout) };
    assert!(
        !ptr.is_null(),
        "aligned_slice allocation failed: size={}, align={}",
        size,
        align
    );
    unsafe { Box::from_raw(slice_from_raw_parts_mut(ptr, size)) }
}

#[inline(always)]
fn align_down(value: usize, align: usize) -> usize {
    debug_assert!(align.is_power_of_two());
    value & !(align - 1)
}

#[inline(always)]
fn align_up(value: usize, align: usize) -> usize {
    debug_assert!(align.is_power_of_two());
    (value + (align - 1)) & !(align - 1)
}

#[inline(always)]
fn normalize_layout(layout: Layout) -> Layout {
    Layout::from_size_align(layout.size().max(1), layout.align()).expect("invalid layout")
}

#[inline(always)]
fn class_index_for_layout(layout: Layout) -> Option<usize> {
    let needed = layout.size().max(1).max(layout.align());
    if needed > MAX_SLAB_SIZE {
        return None;
    }

    CLASS_SIZES.iter().position(|&sz| sz >= needed)
}

#[inline(always)]
fn page_count_for(size: usize) -> usize {
    size.max(1).div_ceil(PAGE_SIZE)
}