use crate::devices::device_discovery::DeviceDiscovery;
use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::devices::device_discovery::{
    BLOCK_DEVICES, CHAR_DEVICES, DeviceNode, DeviceType, SYSTEM_DRIVERS,
};
use crate::print::kprintln;
use crate::sync::MutexLike;
use fdt;
use limine::request::DeviceTreeBlobRequest;
pub mod psci;

#[used]
#[unsafe(link_section = ".limine_requests")]
pub static DTB_REQUEST: DeviceTreeBlobRequest = DeviceTreeBlobRequest::new();

// parse the device tree and match devices to drivers. This should be called after all system drivers have been set up in SYSTEM_DRIVERS
// TODO: ideally we abstract this out, we just give a list of nodes to a function in the shared driver code
// and it handles this business of matching and pushing to the right lists
pub fn parse_devices() {
    // get dtb pointer
    if let Some(resp) = DTB_REQUEST.get_response() {
        let dtb_addr = resp.dtb_ptr();
        let fdt = unsafe {
            fdt::Fdt::from_ptr(dtb_addr as *const u8).expect("Failed to parse device tree blob.")
        };

        // go through all nodes and pass them into all devices, and if any are returned push them to the matched devices list
        for driver in SYSTEM_DRIVERS.lock().iter() {
            for node in fdt.all_nodes() {
                let matched_device = driver.am_i_this(DeviceNode::DTB(node));
                if let Some(device) = matched_device {
                    match device {
                        DeviceType::Block(d) => BLOCK_DEVICES.lock().push(d),
                        DeviceType::Char(d) => CHAR_DEVICES.lock().push(d),
                        DeviceType::Special => {} // they handle themselves.
                    }
                }
            }
        }
    }
    dump_device_tree();
}

pub fn dump_device_tree() {
    if let Some(resp) = DTB_REQUEST.get_response() {
        let dtb_addr = resp.dtb_ptr();
        let fdt = unsafe {
            fdt::Fdt::from_ptr(dtb_addr as *const u8).expect("Failed to parse device tree blob.")
        };
        for node in fdt.all_nodes() {
            kprintln!("Node: {}", node.name);
            for prop in node.properties() {
                if prop.name == "compatible" {
                    kprintln!(
                        "  Compatible: {:?}",
                        prop.as_str()
                            .expect("Failed to read compatible property as string")
                    );
                } else {
                    kprintln!("  Prop: {} = {:?}", prop.name, prop.value);
                }
            }
        }
    }
}

pub fn create_arch_specific_drivers(
    system_drivers: &mut Vec<Box<dyn DeviceDiscovery + Send + Sync>>,
) {
    system_drivers.push(Box::new(psci::PSCIDiscovery));
}
