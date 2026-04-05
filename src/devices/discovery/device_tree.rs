use crate::devices::discovery::{DeviceNode, DeviceType, SYSTEM_DRIVERS};
use crate::print::kprintln;
use crate::sync::MutexLike;
use alloc::vec::Vec;
use core::option::Option;
use fdt::{self};
use limine::request::DeviceTreeBlobRequest;

#[used]
#[unsafe(link_section = ".limine_requests")]
pub static DTB_REQUEST: DeviceTreeBlobRequest = DeviceTreeBlobRequest::new();

// parse the device tree and match devices to drivers. This should be called after all system drivers have been set up in SYSTEM_DRIVERS
// TODO: ideally we abstract this out, we just give a list of nodes to a function in the shared driver code
// and it handles this business of matching and pushing to the right lists
pub fn parse_device_tree() -> Option<Vec<DeviceType>> {
    // get dtb pointer
    let mut matched_devices = Vec::new();
    let resp = DTB_REQUEST.get_response()?;
    let dtb_addr = resp.dtb_ptr();
    let fdt = unsafe {
        fdt::Fdt::from_ptr(dtb_addr as *const u8).expect("Failed to parse device tree blob.")
    };

    // go through all nodes and pass them into all devices, and if any are returned push them to the matched devices list
    for driver in SYSTEM_DRIVERS.lock().iter() {
        for node in fdt.all_nodes() {
            let matched_device = driver.am_i_this(DeviceNode::DTB(node));
            if let Some(device) = matched_device {
                matched_devices.push(device);
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
