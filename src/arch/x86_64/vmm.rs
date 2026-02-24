use core::arch::asm;
use x86_64::{PhysAddr, VirtAddr, structures::paging::{FrameAllocator, FrameDeallocator, Mapper, OffsetPageTable, Page, PageTable, PageTableFlags, PhysFrame, Size4KiB, page::AddressNotAligned}}; // https://docs.rs/x86_64/latest/x86_64/structures/paging/
use crate::{physical_memory::{HHDM_REQUEST, frame_alloc, frame_dealloc}, sync::{IntMutex}};

// ChatGPT told me how to do this trait impl'ing
pub struct FrameAllocatorWrapper {
    pub inner: fn() -> u64
}
unsafe impl FrameAllocator<Size4KiB> for FrameAllocatorWrapper {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        // if our physmem allocator starts returns misaligned frames, we're in big trouble...
        Some(PhysFrame::from_start_address(PhysAddr::new((self.inner)())).ok()?)
    }
}
pub struct FrameDeallocatorWrapper {
    pub inner: fn(u64) -> ()
}

impl FrameDeallocator<Size4KiB> for FrameDeallocatorWrapper {
    unsafe fn deallocate_frame(&mut self, frame: PhysFrame<Size4KiB>) {
        (self.inner)(frame.start_address().as_u64())
    }
}

// used ChatGPT for Rust syntax help
pub fn get_address_space() -> u64 {
    let cr3 : u64;
    unsafe {
        asm!(
            "mov {0}, cr3",
            out(reg) cr3,
        );
    }
    cr3
}

struct VMMProtector; // TODO make cr3-specific
static VMM_PROTECTOR : IntMutex<VMMProtector> = IntMutex::new(VMMProtector{});

// TODO use PAT/PCD/PWT bits?
pub fn vmap(space: u64, vaddr: u64, paddr: u64, user_accessible: bool, executable: bool, writable: bool) {
    // TODO avoid doing this every time somehow?
    let hhdm_offset : u64 = HHDM_REQUEST.get_response().unwrap().offset();
    let mut mapper = unsafe {OffsetPageTable::new(
        &mut *((space + hhdm_offset) as *mut PageTable), 
        VirtAddr::new(hhdm_offset)
    )};

    let mut flags = PageTableFlags::empty();
    flags.insert(PageTableFlags::PRESENT);
    if user_accessible {flags.insert(PageTableFlags::USER_ACCESSIBLE)}
    if writable {flags.insert(PageTableFlags::WRITABLE)}
    if !executable {flags.insert(PageTableFlags::NO_EXECUTE)}

    // there has to be a better way of error handling...
    if let Ok(vpage) = Page::<Size4KiB>::from_start_address(VirtAddr::new(vaddr)) {
        if let Ok(pframe) = PhysFrame::from_start_address(PhysAddr::new(paddr)) {
            if let Ok(toilet) = {
                let _ = VMM_PROTECTOR.lock();
                unsafe {
                    mapper.map_to(
                    vpage,
                    pframe,
                    flags, 
                    &mut FrameAllocatorWrapper{inner: frame_alloc} // TODO allocator wrapper is kinda dumb
            )}} {
                toilet.flush(); // terrific variable name i know
            } else {
                panic!("mapping physical page {} at virtual address {} failed unexpectedly", paddr, vaddr);
            }
        } else {
            panic!("misaligned physical address {} provided", paddr);
        }
    } else {
        panic!("misaligned virtual address {} provided", vaddr);
    }
    
}

pub fn vunmap(space: u64, vaddr: u64) -> u64 {
    let hhdm_offset : u64 = HHDM_REQUEST.get_response().unwrap().offset();
    let mut mapper = unsafe {OffsetPageTable::new(
        &mut *((space + hhdm_offset) as *mut PageTable), 
        VirtAddr::new(hhdm_offset)
    )};

    if let Ok(vpage) = Page::<Size4KiB>::from_start_address(VirtAddr::new(vaddr)) {
        if let Ok((frame, toilet)) = {
            let guard  = VMM_PROTECTOR.lock();
            mapper.unmap(vpage)
        } {
            toilet.flush(); // wow, this seems to handle all the TLB clearing for us, but not the IPI...
            unsafe {FrameDeallocatorWrapper{inner: frame_dealloc}.deallocate_frame(frame)}; // no shared mappings for now
            frame.start_address().as_u64() // returning this will be useful when we allow shared mappings
        } else {
            panic!("unmapping page {} failed unexpectedly", vaddr);
        }
    } else {
        panic!("misaligned virtual address {} provided", vaddr);
    }
}