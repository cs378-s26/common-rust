use crate::print::kprintln;

use super::device_tree::{DeviceInfo, DeviceRegistry};

// called when a device is matched -> receives full DeviceInfo, may write back discovered fields
pub type DriverInitFn = fn(&mut DeviceInfo);

// one row in the driver table
pub struct DriverEntry {
    pub compatible: &'static str,
    pub init: DriverInitFn,
}

// add one DriverEntry here per driver
pub static DRIVERS: &[DriverEntry] = &[
    DriverEntry {
        compatible: "virtio,mmio",
        init: super::drivers::virtio_mmio::virtio_mmio_init,
    },
];

// walk every device and call the first matching driver
pub fn probe_drivers(registry: &mut DeviceRegistry) {
    for dev in &mut registry.devices {
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
