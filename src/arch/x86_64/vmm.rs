use core::arch::asm;

use spin::Mutex;
use x86_64::{
    PhysAddr, VirtAddr,
    structures::paging::{
        FrameAllocator, Mapper, OffsetPageTable, Page, PageTable, PageTableFlags, PhysFrame,
        Size4KiB,
    },
};

use crate::{
    arch::apic,
    memory::{
        physical_memory::{HHDM_REQUEST, PageFrameAllocator, frame_dealloc},
        virtual_memory::PagingOptions,
    },
}; // https://docs.rs/x86_64/latest/x86_64/structures/paging/

// ChatGPT told me how to do this trait impl'ing
struct FrameAllocatorWrapper<'a, P>(&'a mut P);

unsafe impl<P: PageFrameAllocator> FrameAllocator<Size4KiB> for FrameAllocatorWrapper<'_, P> {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        let frame = self.0.alloc_frame()?;
        PhysFrame::from_start_address(PhysAddr::new(frame as u64)).ok()
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

pub fn set_address_space(cr3: u64) {
    unsafe {
        asm!(
            "mov cr3, {0}",
            in(reg) cr3,
        );
    }
}

struct VMMProtector; // TODO make cr3-specific
static VMM_PROTECTOR: Mutex<VMMProtector> = Mutex::new(VMMProtector {});

pub fn vmap<P: PageFrameAllocator>(
    space: u64,
    vaddr: u64,
    paddr: u64,
    options: PagingOptions,
    pmm: &mut P,
) {
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
        unsafe { mapper.map_to(vpage, pframe, flags, &mut FrameAllocatorWrapper(pmm)) }
    }
    .unwrap_or_else(|e| {
        panic!(
            "mapping physical page {:x} at virtual address {:x} failed unexpectedly: {:?}",
            paddr, vaddr, e
        )
    });
    toilet.flush(); // terrific variable name i know
}

pub fn vunmap_internal(space: u64, vaddr: u64, free_frame: bool) -> Option<u64> {
    apic::get_lapic_id();
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
        toilet.flush(); // this handles all the TLB clearing for us, but not the IPI...
        if free_frame {
            frame_dealloc(frame.start_address().as_u64() as usize); // no shared mappings for now
        }
        Some(frame.start_address().as_u64()) // returning this will be useful when we allow shared mappings
    } else {
        None
    }
}

pub fn vunmap(space: u64, vaddr: u64) -> Option<u64> {
    vunmap_internal(space, vaddr, true)
}

pub fn vunmap_no_dealloc(space: u64, vaddr: u64) -> Option<u64> {
    vunmap_internal(space, vaddr, false)
}
