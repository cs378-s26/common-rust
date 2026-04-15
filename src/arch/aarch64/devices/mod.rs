use alloc::{boxed::Box, vec::Vec};

use crate::devices::discovery::DeviceDiscovery;

pub mod a15_gic;

pub mod psci;

pub fn create_arch_specific_drivers(
    system_drivers: &mut Vec<Box<dyn DeviceDiscovery + Send + Sync>>,
) {
    system_drivers.push(Box::new(psci::PSCIDiscovery));
    system_drivers.push(Box::new(a15_gic::GicA15Discovery));
}
