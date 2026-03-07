
use limine::request::DeviceTreeBlobRequest;

#[used]
#[unsafe(link_section = ".limine_requests")]
static FDT_REQUEST: DeviceTreeBlobRequest = DeviceTreeBlobRequest::new();

use alloc::vec::Vec;
use crate::arch::aarch64::SerialCharSink;
use crate::arch::{Arch, ArchTrait};
use crate::physical_memory::HHDM_REQUEST;
use crate::print::{self, kprintln, SERIAL_BACKEND};
use crate::virtual_memory::PagingOptions;
use spin::Once;
use fdt::node::FdtNode;

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
            // if node.name.contains("virtio") {
                // kprintln!("node: {}", node.name);
            // this can be edited to print all properties using node.properties()
            for prop in node.properties() {
                if prop.name == "compatible" {
                    let compat = prop
                        .as_str()
                        .expect("compatible property should be a string");
                    // kprintln!("compatible: {}", compat);
                } else if prop.name == "interrupts" {
                    let ints = prop
                        .value
                        .chunks_exact(4)
                        .map(|chunk| {
                            u32::from_be_bytes(chunk.try_into().expect("chunk should be 4 bytes"))
                        })
                        .collect::<Vec<u32>>();
                    // kprintln!("interrupts: {:?}", ints);
                }
            }
        }
    } else {
        // kprintln!("warn: no response received for FDT request");
    }
}


pub fn parse_devices() {
    if let Some(df) = FDT.get() {
        for node in df.all_nodes() {
            if node.name.contains("virtio") {
                map_virtio_device(node);
            } else if node.name.contains("pl011") {
                init_pl011_uart(node);
            }
        }
    } else {
        // kprintln!("warn: no response received for FDT request");
    }
}

fn init_pl011_uart(node: FdtNode) {

    let hhdm_offset = HHDM_REQUEST.get_response().unwrap().offset() as usize;
    if let Some(mut reg) = node.reg() {
        let base = reg.next().unwrap().starting_address as usize;
        let flags = PagingOptions::PRESENT | PagingOptions::WRITABLE;
        Arch::virtual_map(
            Arch::get_address_space(),
            (base + hhdm_offset) as u64,
            base as u64,
            flags,
        );
        SERIAL_BACKEND.call_once(|| SerialCharSink::open(base + hhdm_offset));
        kprintln!("pls work");
    }


}

fn map_virtio_device(node: FdtNode) {

    let hhdm_offset = HHDM_REQUEST.get_response().unwrap().offset() as usize;
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
    }
}
