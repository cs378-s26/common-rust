pub mod a15_gic;
pub mod uart_pl011;
use crate::devices::device_discovery::{DeviceNode, MATCHED_DEVICES, SYSTEM_DRIVERS};
use crate::print::kprintln;
use crate::sync::MutexLike;
use alloc::boxed::Box;
use fdt;
use limine::request::DeviceTreeBlobRequest;

#[used]
#[unsafe(link_section = ".limine_requests")]
pub static DTB_REQUEST: DeviceTreeBlobRequest = DeviceTreeBlobRequest::new();

// parse the device tree and match devices to drivers. This should be called after all system drivers have been set up in SYSTEM_DRIVERS
pub fn parse_devices() {
    if let Some(resp) = DTB_REQUEST.get_response() {
        let dtb_addr = resp.dtb_ptr();
        let fdt = unsafe {
            fdt::Fdt::from_ptr(dtb_addr as *const u8).expect("Failed to parse device tree blob.")
        };
        for node in fdt.all_nodes() {
            for driver in SYSTEM_DRIVERS.lock().iter() {
                let matched_device = driver.am_i_this(DeviceNode::DTB(node));
                if let Some(device) = matched_device {
                    MATCHED_DEVICES.lock().push(device);
                }
            }
        }
    }
}

pub fn create_arch_specific_drivers() {
    // create drivers for devices that are specific to this architecture, for example aarch64's uart_pl011
    let mut drivers = crate::devices::device_discovery::SYSTEM_DRIVERS.lock();
    drivers.push(Box::new(uart_pl011::UartPl011Discovery));
}

pub fn init_arch_specific_drivers() {}
