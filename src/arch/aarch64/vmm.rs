use core::arch::asm;

use bitflags::bitflags;
use spin::Mutex;

use crate::{
    kprintln,
    memory::{
        physical_memory::{HHDM_REQUEST, frame_alloc, frame_dealloc},
        virtual_memory::PagingOptions,
    },
};

const PTE_ADDR_MASK: u64 = 0x0000FFFFFFFFF000; // bits [47:12] contain the physical address for a page table entry, the rest are flags
const PTE_PER_PAGE: usize = 512; // each page table has 512 entries, since each entry is 8 bytes and page size is 4096 bytes
static VMM_PROTECTOR: Mutex<()> = Mutex::new(()); // TODO make this address space specific

// useful docs for this: https://developer.arm.com/documentation/ddi0601/2025-12/AArch64-Registers/MAIR-EL1--Memory-Attribute-Indirection-Register--EL1-
pub fn configure_vm() {
    // MAIR_EL1 stands for memory attribute indirection register, for each PT entry you select an attribute from this
    // register to determine caching and ordering, this is set up here
    let normal_memory_cacheable = 0xFF;
    let normal_memory_non_cacheable = 0x44 << 8; // each index is 8 bites, so this sets it to index 1
    let device_memory = 0x00 << 16; // nGnRnE, obviously this does nothing in the or below but included here for clarity
    let mair_el1 = normal_memory_cacheable | normal_memory_non_cacheable | device_memory;
    unsafe {
        core::arch::asm!(
            "msr MAIR_EL1, {0:x}",
            "isb",
            in(reg) mair_el1,
            options(nostack, preserves_flags)
        );
    }
    kprintln!("Configured VM with MAIR_EL1 = {:#x}", mair_el1);
}

pub fn get_phys_addr(vaddr: u64, space: u64) -> Option<u64> {
    let hhdm_offset = HHDM_REQUEST.get_response().unwrap().offset() as usize;

    let index_0 = ((vaddr >> 39) & 0x1FF) as usize;
    let index_1 = ((vaddr >> 30) & 0x1FF) as usize;
    let index_2 = ((vaddr >> 21) & 0x1FF) as usize;
    let index_3 = ((vaddr >> 12) & 0x1FF) as usize;

    let pt_base = (space & !0xFFF) + hhdm_offset as u64;
    unsafe {
        let _ = VMM_PROTECTOR.lock();
        let l0: &[u64] = core::slice::from_raw_parts(pt_base as *const u64, PTE_PER_PAGE);
        if (l0[index_0] & 0b11) != 0b11 {
            return None;
        }

        let l1_phys = l0[index_0] & PTE_ADDR_MASK;
        let l1: &[u64] =
            core::slice::from_raw_parts((l1_phys + hhdm_offset as u64) as *const u64, PTE_PER_PAGE);
        if (l1[index_1] & 0b11) != 0b11 {
            return None;
        }

        let l2_phys = l1[index_1] & PTE_ADDR_MASK;
        let l2: &[u64] =
            core::slice::from_raw_parts((l2_phys + hhdm_offset as u64) as *const u64, PTE_PER_PAGE);
        if (l2[index_2] & 0b11) != 0b11 {
            return None;
        }

        let l3_phys = l2[index_2] & PTE_ADDR_MASK;
        let l3: &[u64] =
            core::slice::from_raw_parts((l3_phys + hhdm_offset as u64) as *const u64, PTE_PER_PAGE);
        if (l3[index_3] & 0b1) == 0 {
            return None;
        }

        Some((l3[index_3] & PTE_ADDR_MASK) | (vaddr & 0xFFF))
    }
}

pub fn phys_to_virt(paddr: u64) -> u64 {
    paddr + HHDM_REQUEST.get_response().unwrap().offset()
}

// TODO allow for shared mappings and write-through caching
bitflags! {
    pub struct PageTableEntryFlags: u64 {
        const VALID = 1 << 0;
        const TABLE = 1 << 1; // for levels 0-2 this means entry points to valid page table, for level 3 this means valid page mapping

        const NORMAL_MEMORY_CACHEABLE = 0b00 << 2; // MAIR_EL1 attr index for normal memory
        const NORMAL_MEMORY_NON_CACHEABLE = 0b01 << 2; // MAIR_EL1 attr index for normal memory, but uncachable
        const DEVICE_MEMORY = 0b10 << 2; // MAIR_EL1 attr index for device memory, specifically nGnRnE

        // access permission (AP) bits:
        const RW_EL1 = 0b00 << 6; // Read/Write at EL1, no access for EL0
        const RW_EL0 = 0b01 << 6; // Read/Write at EL0
        const RO_EL1 = 0b10 << 6; // Read-Only at EL1, no access for EL0
        const RO_EL0 = 0b11 << 6; // Read-Only at EL0

        const AF = 1 << 10;   // Access Flag
        const UXN = 1 << 54;  // Unprivileged Execute Never
        const PXN = 1 << 53;  // Privileged Execute Never
    }
}

pub fn get_kernel_address_space() -> u64 {
    let mut ttbr1: u64; // translation table base register 1, used for kernel space mappings, i.e starting at 0xffff_0000_...

    unsafe {
        asm!(
            "mrs {}, ttbr1_el1",
            out(reg) ttbr1,
            options(nomem, nostack, preserves_flags)
        );
    }
    ttbr1
}

pub fn get_user_address_space() -> u64 {
    let mut ttbr0: u64; // translation table base register 0, used for user space mappings, i.e below 0xffff_0000_...

    unsafe {
        asm!(
            "mrs {}, ttbr0_el1",
            out(reg) ttbr0,
            options(nomem, nostack, preserves_flags)
        );
    }
    ttbr0
}

pub fn set_user_address_space(space: u64) {
    // TODO: Currently, this is just a hack so that we don't flush the
    // TLB too much. In the future, we should implement process ID
    // tagging.
    if space == get_user_address_space() {
        return;
    }
    unsafe {
        asm!(
            "msr ttbr0_el1, {}",
            "tlbi vmalle1is",
            "dsb ishst",
            in(reg) space,
            options(nomem, nostack, preserves_flags)
        );
    }
}

// map a virtual address to a physical address in the given address space, with the given paging options
// if calling this directly, make sure to call tlb shootdown to prevent any caching issues with overwriting
pub fn vmap(space: u64, vaddr: u64, paddr: u64, options: PagingOptions) {
    let hhdm_offset = HHDM_REQUEST.get_response().unwrap().offset() as usize;

    // back in my day, we didn't have no fancy x86 crate to parse our pages, we did it
    // manually with bit shifts and masks, and we liked it that way
    // TODO add a helper struct/function to make this less ugly
    let index_0 = ((vaddr >> 39) & 0x1FF) as usize;
    let index_1 = ((vaddr >> 30) & 0x1FF) as usize;
    let index_2 = ((vaddr >> 21) & 0x1FF) as usize;
    let index_3 = ((vaddr >> 12) & 0x1FF) as usize;

    let pt_base = (space & !0xFFF) + hhdm_offset as u64;
    unsafe {
        let _ = VMM_PROTECTOR.lock();
        // TODO make a function to reduce repeated code here
        let l0: &mut [u64] = core::slice::from_raw_parts_mut(pt_base as *mut u64, PTE_PER_PAGE);

        let l1_phys = ensure_next_table(&mut l0[index_0], hhdm_offset);
        let l1: &mut [u64] =
            core::slice::from_raw_parts_mut((l1_phys + hhdm_offset) as *mut u64, PTE_PER_PAGE);

        let l2_phys = ensure_next_table(&mut l1[index_1], hhdm_offset);
        let l2: &mut [u64] =
            core::slice::from_raw_parts_mut((l2_phys + hhdm_offset) as *mut u64, PTE_PER_PAGE);

        let l3_phys = ensure_next_table(&mut l2[index_2], hhdm_offset);
        let l3: &mut [u64] =
            core::slice::from_raw_parts_mut((l3_phys + hhdm_offset) as *mut u64, PTE_PER_PAGE);

        l3[index_3] = (paddr & PTE_ADDR_MASK) | create_aarch64_attributes(options);
    }
}

// create the attribute bits for a page table entry based on the given paging options
// Translations here are a bit coarse grained, we may want to change PagingOptions
// later to give more control
fn create_aarch64_attributes(options: PagingOptions) -> u64 {
    let mut attr = 0;

    attr |= PageTableEntryFlags::AF.bits(); // set Access Flag, so we don't get a permission fault on access before we set it in the page tables
    if options.contains(PagingOptions::PRESENT) {
        attr |= PageTableEntryFlags::VALID.bits();
        attr |= PageTableEntryFlags::TABLE.bits();
    }
    if options.contains(PagingOptions::WRITABLE) && options.contains(PagingOptions::USER_ACCESSIBLE)
    {
        attr |= PageTableEntryFlags::RW_EL0.bits();
    } else if options.contains(PagingOptions::WRITABLE) {
        attr |= PageTableEntryFlags::RW_EL1.bits();
    } else if options.contains(PagingOptions::USER_ACCESSIBLE) {
        attr |= PageTableEntryFlags::RO_EL0.bits();
    } else {
        attr |= PageTableEntryFlags::RO_EL1.bits();
    }
    if !options.contains(PagingOptions::EXECUTABLE) {
        attr |= PageTableEntryFlags::UXN.bits() | PageTableEntryFlags::PXN.bits(); // Unprivileged and Privileged Execute Never
    }
    if options.contains(PagingOptions::DEVICE_MEMORY) {
        attr |= PageTableEntryFlags::DEVICE_MEMORY.bits();
    } else if options.contains(PagingOptions::CACHEABLE) {
        attr |= PageTableEntryFlags::NORMAL_MEMORY_CACHEABLE.bits();
    } else {
        attr |= PageTableEntryFlags::NORMAL_MEMORY_NON_CACHEABLE.bits();
    }

    attr
}

pub fn tlb_shootdown(vaddr: u64) {
    // TODO right now this is assuming we are invalidating a global address, like hhdm or mmio, the
    // pattern will look different for non-global addresses, like user pages
    // pattern is documented here: https://developer.arm.com/documentation/ddi0487/maa/-Part-K-Appendixes/-Appendix-K11-Barrier-Litmus-Tests/-K11-5-Cache-and-TLB-maintenance-instructions-and-barriers/-K11-5-3-TLB-maintenance-instructions-and-barriers
    // tlbi ops here: https://developer.arm.com/documentation/ddi0601/2025-12/AArch64-Instructions

    let page_size_shift: u64 = 12; // log2 of page size

    unsafe {
        asm!(
            // data synchronization barrier, inner shareable domain, st means store. This ensures
            // the write to the page table will be seen before the TLB shootdown
            "dsb ishst",
            "tlbi vaae1is, {}", // invalidate the virtual address in the TLB for EL1 and the inner shareable domain, ensuring all cores see it
            "dsb ish", // stronger synch, ensure tlb shootdown is done before any other memory accesses
            "isb", // ensure next instructions see updates to page tables and tlb
            in(reg) (vaddr >> page_size_shift), // pass in page number
            options(nostack, preserves_flags)
        )
    }
}

// make sure a pt entry contains a valid next-level table, allocating one if necessary,
// and return the physical address of the next-level table
fn ensure_next_table(entry: &mut u64, hhdm_offset: usize) -> usize {
    // these bits check if the entry is valid and points to a next-level table for levels 0-2
    if (*entry & 0b11) == 0 {
        let new_table_phys = frame_alloc();

        // Zero the freshly allocated page table.
        unsafe {
            core::ptr::write_bytes((new_table_phys + hhdm_offset) as *mut u8, 0, 4096);
        }

        // For a next-level table descriptor on AArch64:
        // bits [1:0] = 0b11 means valid table descriptor (for levels 0-2 here)
        *entry = (new_table_phys as u64 & PTE_ADDR_MASK) | 0b11;
    }

    (*entry & PTE_ADDR_MASK) as usize
}

// unmap a virtual address in the given address space, returning the physical address that was mapped there if it was mapped
fn vunmap_internal(space: u64, vaddr: u64, free_frame: bool) -> Option<u64> {
    let hhdm_offset = HHDM_REQUEST.get_response().unwrap().offset() as usize;

    let index_0 = ((vaddr >> 39) & 0x1FF) as usize;
    let index_1 = ((vaddr >> 30) & 0x1FF) as usize;
    let index_2 = ((vaddr >> 21) & 0x1FF) as usize;
    let index_3 = ((vaddr >> 12) & 0x1FF) as usize;

    let pt_base = (space & !0xFFF) + hhdm_offset as u64;
    unsafe {
        let _ = VMM_PROTECTOR.lock();
        let l0: &mut [u64] = core::slice::from_raw_parts_mut(pt_base as *mut u64, PTE_PER_PAGE);
        if (l0[index_0] & 0b11) != 0b11 {
            return None; // entry not valid or not a table
        }
        let l1_phys = (l0[index_0] & PTE_ADDR_MASK) as usize;
        let l1: &mut [u64] =
            core::slice::from_raw_parts_mut((l1_phys + hhdm_offset) as *mut u64, PTE_PER_PAGE);

        if (l1[index_1] & 0b11) != 0b11 {
            return None; // entry not valid or not a table
        }
        let l2_phys = (l1[index_1] & PTE_ADDR_MASK) as usize;
        let l2: &mut [u64] =
            core::slice::from_raw_parts_mut((l2_phys + hhdm_offset) as *mut u64, PTE_PER_PAGE);

        if (l2[index_2] & 0b11) != 0b11 {
            return None; // entry not valid or not a table
        }
        let l3_phys = (l2[index_2] & PTE_ADDR_MASK) as usize;
        let l3: &mut [u64] =
            core::slice::from_raw_parts_mut((l3_phys + hhdm_offset) as *mut u64, PTE_PER_PAGE);
        if (l3[index_3] & 0b11) == 0 {
            return None; // entry not valid
        }

        let paddr = l3[index_3] & PTE_ADDR_MASK;
        // Clear the page table entry to unmap it
        l3[index_3] = 0;
        free_unused_tables(vaddr, l0, l1, l2, l3);
        if free_frame {
            frame_dealloc(paddr as usize);
        }
        Some(paddr)
    }
}

pub fn vunmap(space: u64, vaddr: u64) -> Option<u64> {
    vunmap_internal(space, vaddr, true)
}

pub fn vunmap_no_dealloc(space: u64, vaddr: u64) -> Option<u64> {
    vunmap_internal(space, vaddr, false)
}

fn free_unused_tables(vaddr: u64, l0: &mut [u64], l1: &mut [u64], l2: &mut [u64], l3: &mut [u64]) {
    let hhdm_offset = HHDM_REQUEST.get_response().unwrap().offset() as usize;

    let index_0 = ((vaddr >> 39) & 0x1FF) as usize;
    let index_1 = ((vaddr >> 30) & 0x1FF) as usize;
    let index_2 = ((vaddr >> 21) & 0x1FF) as usize;

    for entry in l3.iter().take(PTE_PER_PAGE) {
        if (entry & 0b11) != 0 {
            return; // valid entry in this table, can't free
        }
    }
    l2[index_2] = 0;
    frame_dealloc((l3.as_mut_ptr() as usize) - hhdm_offset);

    for entry in l2.iter().take(PTE_PER_PAGE) {
        if (entry & 0b11) != 0 {
            return; // valid entry in this table, can't free
        }
    }
    l1[index_1] = 0;
    frame_dealloc((l2.as_mut_ptr() as usize) - hhdm_offset);

    for entry in l1.iter().take(PTE_PER_PAGE) {
        if (entry & 0b11) != 0 {
            return; // valid entry in this table, can't free
        }
    }
    l0[index_0] = 0;
    frame_dealloc((l1.as_mut_ptr() as usize) - hhdm_offset);
}
