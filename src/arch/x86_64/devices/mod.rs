use alloc::{boxed::Box, vec::Vec};
use super::ioapic;

use crate::devices::discovery::DeviceDiscovery;

pub fn create_arch_specific_drivers(SYSTEM_DRIVERS: &mut Vec<Box<dyn DeviceDiscovery + Send + Sync>>) {
    SYSTEM_DRIVERS.push(Box::new(ioapic::discovery{}));
}
