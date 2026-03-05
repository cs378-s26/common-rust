use limine::request::DeviceTreeBlobRequest;

#[used]
#[unsafe(link_section = ".limine_requests")]
static FDT_REQUEST: DeviceTreeBlobRequest = DeviceTreeBlobRequest::new();

use alloc::vec::Vec;
use kernel_common::arch::{Arch, ArchTrait};
use kernel_common::physical_memory::HHDM_REQUEST;
use kernel_common::print::{self, kprintln};
use kernel_common::virtual_memory::PagingOptions;
use spin::Once;
use super::virtio_blk::init_virtio_blk;

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
                    let compat = prop
                        .as_str()
                        .expect("compatible property should be a string");
                    kprintln!("compatible: {}", compat);
                } else if prop.name == "interrupts" {
                    let ints = prop
                        .value
                        .chunks_exact(4)
                        .map(|chunk| {
                            u32::from_be_bytes(chunk.try_into().expect("chunk should be 4 bytes"))
                        })
                        .collect::<Vec<u32>>();
                    kprintln!("interrupts: {:?}", ints);
                }
            }
        }
    } else {
        kprintln!("warn: no response received for FDT request");
    }
}

pub fn map_virtio_devices() {
    kprintln!("Mapping virtio devices...");
    let hhdm_offset = HHDM_REQUEST.get_response().unwrap().offset() as usize;
    if let Some(dt) = FDT.get() {
        for node in dt.all_nodes() {
            // map virtio devices, use mmio
            if node.name.contains("virtio") {
                if let Some(mut reg) = node.reg() {
                    let base = reg.next().unwrap().starting_address as usize;
                    // map the first 4KB of the device's MMIO region
                    // NOTE: this is device memory because it does not set the cacheable bit
                    let flags = PagingOptions::PRESENT | PagingOptions::WRITABLE;
                    Arch::virtual_map(
                        Arch::get_address_space(),
                        (base + hhdm_offset) as u64,
                        base as u64,
                        flags,
                    );
                    let id =
                        unsafe { core::ptr::read_volatile((base + hhdm_offset + 0x8) as *const u32) }; // test read

                    kprintln!("Mapped virtio device at {:#x}, id: {:#x}", base, id);
                    if id == 2 {
                        init_virtio_blk(base + hhdm_offset, 512);
                    }
                }
            }
        }
    } else {
        kprintln!("FDT not initialized, cannot map virtio devices");
    }
}
