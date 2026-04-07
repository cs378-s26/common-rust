use crate::devices::discovery::{DeviceNode, DeviceType, SYSTEM_DRIVERS};
use crate::print::kprintln;
use alloc::vec::Vec;
use core::option::Option;
use fdt::{self};
use limine::request::DeviceTreeBlobRequest;

#[used]
#[unsafe(link_section = ".limine_requests")]
pub static DTB_REQUEST: DeviceTreeBlobRequest = DeviceTreeBlobRequest::new();

pub fn parse_device_tree() -> Option<Vec<DeviceType>> {
    // get dtb pointer
    let mut matched_devices = Vec::new();
    let resp = DTB_REQUEST.get_response()?;
    let dtb_addr = resp.dtb_ptr();
    let fdt = unsafe {
        fdt::Fdt::from_ptr(dtb_addr as *const u8).expect("Failed to parse device tree blob.")
    };

    // Walk nodes outermost so the first matching discovery driver claims each node.
    for node in fdt.all_nodes() {
        for driver in SYSTEM_DRIVERS.iter() {
            let matched_device = driver.am_i_this(DeviceNode::DTB(node));
            if let Some(devices) = matched_device {
                matched_devices.extend(devices);
                break;
            }
        }
    }
    Some(matched_devices)
}

#[allow(dead_code)]
pub fn dump_device_tree() {
    if let Some(resp) = DTB_REQUEST.get_response() {
        let dtb_addr = resp.dtb_ptr();
        let fdt = unsafe {
            fdt::Fdt::from_ptr(dtb_addr as *const u8).expect("Failed to parse device tree blob.")
        };
        kprintln!("Device Tree:");
        for node in fdt.all_nodes() {
            kprintln!("Node: {}", node.name);
            for prop in node.properties() {
                if prop.name == "compatible" {
                    kprintln!(
                        "  Compatible: {:?}",
                        prop.as_str()
                            .expect("Failed to read compatible string from device tree property.")
                    );
                } else {
                    kprintln!("  Property: {}", prop.name); // printing value depends on the type
                }
            }
        }
    } else {
        kprintln!("No device tree found.");
    }
}
