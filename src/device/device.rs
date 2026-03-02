use limine::request::DeviceTreeBlobRequest;

#[used]
#[unsafe(link_section = ".limine_requests")]
static FDT_REQUEST: DeviceTreeBlobRequest = DeviceTreeBlobRequest::new();

use spin::Once;
use kernel_common::print::kprintln;
use alloc::vec::Vec;
use crate::device::page_table_utils::{create_mapping_for_phys_address, HHDM_OFFSET};

static FDT: Once<fdt::Fdt<'static>> = Once::new();

pub fn init_device_tree() {
    if let Some(res) = FDT_REQUEST.get_response() {
        let device_tree =
            unsafe { fdt::Fdt::from_ptr(res.dtb_ptr() as *const u8).expect("invalid device tree") };
        FDT.call_once(|| device_tree);
    } else {
        kprintln!("warn: no response received for FDT request");
    }
}

pub fn print_device_tree() {

    if let Some(device_tree) = FDT.get() {
        for node in device_tree.all_nodes() {
            if node.name.contains("virtio") {
                kprintln!("node: {}", node.name);
            }
            // this can be edited to print all properties using node.properties()
            for prop in node.properties() {
                if prop.name == "compatible" {
                    let compat = prop.as_str().expect("compatible property should be a string");
                    kprintln!("compatible: {}", compat);
                } else if prop.name == "interrupts" {
                    let ints = prop.value.chunks_exact(4).map(|chunk| {
                        u32::from_be_bytes(chunk.try_into().expect("chunk should be 4 bytes"))
                    }).collect::<Vec<u32>>();
                    kprintln!("interrupts: {:?}", ints);
                }
            }
        }
    } else {
        kprintln!("warn: no response received for FDT request");
    }
}

pub fn map_virtio_devices() {
    let hhdm_offset = *HHDM_OFFSET.get().expect("HHDM offset not set");
    if let Some(dt) = FDT.get() {
        for node in dt.all_nodes() {
            // map virtio devices, use mmio
            if node.name.contains("virtio") {
                if let Some(mut reg) = node.reg() {
                    let base = reg.next().unwrap().starting_address as usize;
                    let size = reg.next().unwrap().size;
                    if let Some(s) = size {
                         kprintln!("virtio device at {:#x}, size: {:#x}", base, s);
                    } else {
                        kprintln!("virtio device at {:#x}, size unknown", base);
                    } 
                    create_mapping_for_phys_address(base);
                    let id = unsafe { core::ptr::read_volatile((base + hhdm_offset +0x8) as *const u32)}; // test read
                    kprintln!("Mapped virtio device at {:#x}, magic num: {:#x}", base, id);
                    
                }
            }
        }
    } else {
        kprintln!("FDT not initialized, cannot map virtio devices");
    }
}