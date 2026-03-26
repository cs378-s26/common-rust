use crate::{
    physical_memory::{HHDM_REQUEST, frame_alloc, frame_dealloc},
    virtual_memory::PagingOptions,
};
use core::arch::asm;
use spin::Mutex;
use x86_64::{
    PhysAddr, VirtAddr,
    structures::paging::{
        FrameAllocator, FrameDeallocator, Mapper, OffsetPageTable, Page, PageTable, PageTableFlags,
        PhysFrame, Size4KiB,
    },
}; // https://docs.rs/x86_64/latest/x86_64/structures/paging/

// ChatGPT told me how to do this trait impl'ing
pub struct FrameAllocatorWrapper {
    pub inner: fn() -> usize,
}
unsafe impl FrameAllocator<Size4KiB> for FrameAllocatorWrapper {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        // if our physmem allocator starts returning misaligned frames, we're in big trouble...
        PhysFrame::from_start_address(PhysAddr::new((self.inner)() as u64)).ok()
    }
}

pub struct FrameDeallocatorWrapper {
    pub inner: fn(usize) -> (),
}

impl FrameDeallocator<Size4KiB> for FrameDeallocatorWrapper {
    unsafe fn deallocate_frame(&mut self, frame: PhysFrame<Size4KiB>) {
        (self.inner)(frame.start_address().as_u64() as usize)
    }
}

// used ChatGPT for Rust syntax help
pub fn get_address_space() -> u64 {
    let cr3: u64;
    unsafe {
        asm!(
            "mov {0}, cr3",
            out(reg) cr3,
        );
    }
    cr3
}

// TODO allocator wrapper is kinda dumb

struct VMMProtector; // TODO make cr3-specific
static VMM_PROTECTOR: Mutex<VMMProtector> = Mutex::new(VMMProtector {});

pub fn vmap(space: u64, vaddr: u64, paddr: u64, options: PagingOptions) {
    // TODO avoid doing this every time somehow?
    let hhdm_offset: u64 = HHDM_REQUEST.get_response().unwrap().offset();
    let mut mapper = unsafe {
        OffsetPageTable::new(
            &mut *((space + hhdm_offset) as *mut PageTable),
            VirtAddr::new(hhdm_offset),
        )
    };

    let mut flags = PageTableFlags::empty();
    if options.contains(PagingOptions::PRESENT) {
        flags.insert(PageTableFlags::PRESENT)
    };
    if options.contains(PagingOptions::USER_ACCESSIBLE) {
        flags.insert(PageTableFlags::USER_ACCESSIBLE)
    };
    if options.contains(PagingOptions::WRITABLE) {
        flags.insert(PageTableFlags::WRITABLE)
    };
    if options.contains(PagingOptions::GLOBAL) {
        flags.insert(PageTableFlags::GLOBAL)
    };
    if !options.contains(PagingOptions::EXECUTABLE) {
        flags.insert(PageTableFlags::NO_EXECUTE)
    };
    if !options.contains(PagingOptions::CACHEABLE) {
        flags.insert(PageTableFlags::NO_CACHE)
    };
    if options.contains(PagingOptions::WRITE_THROUGH) {
        flags.insert(PageTableFlags::WRITE_THROUGH)
    };

    // there has to be a better way of error handling...
    let vpage = Page::<Size4KiB>::from_start_address(VirtAddr::new(vaddr))
        .unwrap_or_else(|_| panic!("misaligned virtual address {:x} to vmap", vaddr));
    let pframe = PhysFrame::<Size4KiB>::from_start_address(PhysAddr::new(paddr))
        .unwrap_or_else(|_| panic!("misaligned physical address {:x} to vmap", paddr));
    let toilet = {
        let _ = VMM_PROTECTOR.lock();
        unsafe {
            mapper.map_to(
                vpage,
                pframe,
                flags,
                &mut FrameAllocatorWrapper { inner: frame_alloc },
            )
        }
    }
    .unwrap_or_else(|_| {
        panic!(
            "mapping physical page {:x} at virtual address {:x} failed unexpectedly",
            paddr, vaddr
        )
    });
    toilet.flush(); // terrific variable name i know
}

pub fn vunmap(space: u64, vaddr: u64) -> Option<u64> {
    let hhdm_offset: u64 = HHDM_REQUEST.get_response().unwrap().offset();
    let mut mapper = unsafe {
        OffsetPageTable::new(
            &mut *((space + hhdm_offset) as *mut PageTable),
            VirtAddr::new(hhdm_offset),
        )
    };

    let vpage = Page::<Size4KiB>::from_start_address(VirtAddr::new(vaddr))
        .unwrap_or_else(|_| panic!("misaligned virtual address {:x} to vunmap", vaddr));
    if let Ok((frame, toilet)) = {
        let _ = VMM_PROTECTOR.lock();
        mapper.unmap(vpage)
    } {
        toilet.flush(); // this handles all the TLB clearing for us, but not the IPI... // TODO! TLB shootdown
        unsafe {
            FrameDeallocatorWrapper {
                inner: frame_dealloc,
            }
            .deallocate_frame(frame)
        }; // no shared mappings for now
        Some(frame.start_address().as_u64()) // returning this will be useful when we allow shared mappings
    } else {
        None
    }
}

/// Unmaps a page without freeing the underlying physical frame.
/// Used for MMIO mappings where physical addresses belong to hardware, not RAM.
pub fn vunmap_no_dealloc(space: u64, vaddr: u64) -> Option<u64> {
    let hhdm_offset: u64 = HHDM_REQUEST.get_response().unwrap().offset();
    let mut mapper = unsafe {
        OffsetPageTable::new(
            &mut *((space + hhdm_offset) as *mut PageTable),
            VirtAddr::new(hhdm_offset),
        )
    };

    let vpage = Page::<Size4KiB>::from_start_address(VirtAddr::new(vaddr)).unwrap_or_else(|_| {
        panic!(
            "misaligned virtual address {:x} to vunmap_no_dealloc",
            vaddr
        )
    });
    if let Ok((frame, toilet)) = {
        let _ = VMM_PROTECTOR.lock();
        mapper.unmap(vpage)
    } {
        toilet.flush();
        Some(frame.start_address().as_u64())
    } else {
        None
    }
}
