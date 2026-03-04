use limine::request::HhdmRequest;
use spin::Once;
use core::arch::asm;
use alloc::boxed::Box;

//TODO some of this needs to be in arch, 

#[used]
#[unsafe(link_section = ".limine_requests")]
static HHDM_REQUEST: HhdmRequest = HhdmRequest::new();

pub static HHDM_OFFSET: Once<usize> = Once::new();

pub fn set_hhdm_offset() {
    let offset = HHDM_REQUEST
        .get_response()
        .expect("hhdm request failed")
        .offset();
    HHDM_OFFSET.call_once(|| offset as usize);
}

#[repr(align(4096))]
pub struct PageTable {
    entries: [u64; 512],
}


// TODO figure out what parts of this are arch dependent

pub fn get_phys_address(virt_addr: usize) -> Option<usize> {
    let ttbr1_el1: usize;
    let hhdm_offset = *HHDM_OFFSET.get().expect("HHDM offset not set");
    unsafe { 
        asm!(
            "mrs {0}, ttbr1_el1",
            out(reg) ttbr1_el1
        );
    }

    // indices for each level of page table, 9 bits each for 4 levels = 36 bits, plus 12 bits for page offset = 48 bits total
    let index_0 = (virt_addr >> 39) & 0x1FF; // bits 39-47
    let index_1 = (virt_addr >> 30) & 0x1FF; // bits 30-38
    let index_2 = (virt_addr >> 21) & 0x1FF; // bits 21-29
    let index_3 = (virt_addr >> 12) & 0x1FF; // bits 12-20

    // mask out bottom 12 bits to get base address of page tables, since page tables are 4KiB aligned. 2^12 = 4096
    let page_table_base = (ttbr1_el1 & !0xfff) + hhdm_offset; // add hhdm offset for qemu virt

    unsafe {
        let pt_0 = core::slice::from_raw_parts(page_table_base as *const u64, 512);
        let entry_0 = pt_0[index_0];
        if entry_0 & 0b11 == 0 {
            return None;
        }
        let entry_1_addr = (entry_0 & 0x0000_FFFF_FFFF_F000) + hhdm_offset as u64;
        let pt_1 = core::slice::from_raw_parts(entry_1_addr as *const u64, 512);
        let entry_1 = pt_1[index_1];
        if entry_1 & 0b11 == 0 {
            return None;
        }
        let entry_2_addr = (entry_1 & 0x0000_FFFF_FFFF_F000) + hhdm_offset as u64;
        let pt_2 = core::slice::from_raw_parts(entry_2_addr as *const u64, 512);
        let entry_2 = pt_2[index_2];
        if entry_2 & 0b11 == 0 {
            return None;
        }
        let entry_3_addr = (entry_2 & 0x0000_FFFF_FFFF_F000) + hhdm_offset as u64;
        let pt_3 = core::slice::from_raw_parts(entry_3_addr as *const u64, 512);
        let entry_3 = pt_3[index_3];
        if entry_3 & 0b11 == 0b00 {
            return None;
        }
        let phys_addr = (entry_3 & 0x0000_FFFF_FFFF_F000) + (virt_addr & 0xfff) as u64;
        return Some(phys_addr as usize);
    }
}

// this function returns a virtual address, not a physical address
fn create_page_table() -> usize {
    // create a new page table in the heap and return a pointer to it
    let page_table = Box::leak(Box::new(PageTable { entries: [0; 512] }));
    let addr = page_table as *mut PageTable as usize;
    assert!(addr % 4096 == 0, "Page table is not 4KiB aligned");
    return addr;
}

// creates a mapping for the given physical address with device memory attributes
// used rn for mmio
pub fn create_mapping_for_phys_address(phys_addr: usize) {
    let hhdm_offset = *HHDM_OFFSET.get().expect("HHDM offset not set");
    let kernel_va = phys_addr + hhdm_offset;

    // get base address of page table
    let ttbr1_el1: u64;
    unsafe {
        asm!(
            "mrs {0}, ttbr1_el1",
            out(reg) ttbr1_el1
        );
    }

    // indices for each level of page table, 9 bits each for 4 levels = 36 bits, plus 12 bits for page offset = 48 bits total
    let index_0 = (kernel_va >> 39) & 0x1FF; // bits 39-47
    let index_1 = (kernel_va >> 30) & 0x1FF; // bits 30-38
    let index_2 = (kernel_va >> 21) & 0x1FF; // bits 21-29
    let index_3 = (kernel_va >> 12) & 0x1FF; // bits 12-20

    // mask out bottom 12 bits to get base address of page tables, since page tables are 4KiB aligned. 2^12 = 4096
    let page_table_base = (ttbr1_el1 & !0xfff) + hhdm_offset as u64; // add hhdm offset for qemu virt

    unsafe {
        let pt_0: &mut [u64] = core::slice::from_raw_parts_mut(page_table_base as *mut u64, 512);
        let mut entry_0 = pt_0[index_0 as usize];
        if entry_0 & 0b11 == 0 {
            let new_pt = create_page_table();
            let phys_addr = get_phys_address(new_pt);
            match phys_addr {
                Some(pa) => {
                    pt_0[index_0] = (pa as u64) | 0b11; // present and writable
                }
                None => {
                    panic!("Failed to get physical address of new page table");
                }
            }
            entry_0 = pt_0[index_0];
        }

        let entry_1_addr = (entry_0 & 0x0000_FFFF_FFFF_F000) + hhdm_offset as u64;
        let pt_1: &mut [u64] = core::slice::from_raw_parts_mut(entry_1_addr as *mut u64, 512);
        let mut entry_1 = pt_1[index_1 as usize];
        if entry_1 & 0b11 == 0 {
            let new_pt = create_page_table();
            let phys_addr = get_phys_address(new_pt);
            match phys_addr {
                Some(pa) => {
                    pt_1[index_1] = (pa as u64) | 0b11; // present and writable
                }
                None => {
                    panic!("Failed to get physical address of new page table");
                }
            }

            entry_1 = pt_1[index_1];
        }

        let entry_2_addr = (entry_1 & 0x0000_FFFF_FFFF_F000) + hhdm_offset as u64;
        let pt_2: &mut [u64] = core::slice::from_raw_parts_mut(entry_2_addr as *mut u64, 512);
        let mut entry_2 = pt_2[index_2 as usize];
        if entry_2 & 0b11 == 0 {
            let new_pt = create_page_table();
            let phys_addr = get_phys_address(new_pt);
            match phys_addr {
                Some(pa) => {
                    pt_2[index_2] = (pa as u64) | 0b11; // present and writable
                }
                None => {
                    panic!("Failed to get physical address of new page table");
                }
            }
            entry_2 = pt_2[index_2];
        }

        let entry_3_addr = (entry_2 & 0x0000_FFFF_FFFF_F000) + hhdm_offset as u64;
        let pt_3: &mut [u64] = core::slice::from_raw_parts_mut(entry_3_addr as *mut u64, 512);
        let entry_3 = pt_3[index_3];

        
        // TODO this should all be arch specific
        if entry_3 & 0b11 == 0 {
            let new_pt = phys_addr & !0xfff; // convert to physical address for page table entry. Align to 4096 bytes
            let desc = (new_pt as u64)
            | (0b11 << 2) // set it to MAIR attribute 3, in this case device memory. 
            | 0b11 // present and writable
            | (1 << 10) 
            | (1 << 54)
            | (1 << 53);
            pt_3[index_3 as usize] = desc;
        }
    }
}