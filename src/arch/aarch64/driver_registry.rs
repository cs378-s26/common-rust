use crate::print::kprintln;

use super::device_tree::{DeviceInfo, DeviceRegistry};

// called when a device is matched -> receives full DeviceInfo
pub type DriverInitFn = fn(&DeviceInfo);

// one row in the driver table
pub struct DriverEntry {
    pub compatible: &'static str,
    pub init: DriverInitFn,
}

// add one DriverEntry here per driver
pub static DRIVERS: &[DriverEntry] = &[
    // TODO add virtio-mmio here
];

// walk every device and call the first matching driver
pub fn probe_drivers(registry: &DeviceRegistry) {
    for dev in &registry.devices {
        let mut matched = false;
        'outer: for compat in &dev.compatible {
            for entry in DRIVERS {
                if *compat == entry.compatible {
                    // TODO remove before PR
                    kprintln!("driver: {} matched by '{}'", dev.name, entry.compatible);
                    (entry.init)(dev);
                    matched = true;
                    break 'outer;
                }
            }
        }
        if !matched && !dev.compatible.is_empty() {
            // TODO remove before PR
            kprintln!("driver: {} no driver for '{}'", dev.name, dev.compatible[0]);
        }
    }
}
