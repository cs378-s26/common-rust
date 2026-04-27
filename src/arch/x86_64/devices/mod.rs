use alloc::{boxed::Box, vec::Vec};

use super::ioapic;
use crate::devices::discovery::DeviceDiscovery;

pub fn create_arch_specific_drivers(
    system_drivers: &mut Vec<Box<dyn DeviceDiscovery + Send + Sync>>,
) {
    system_drivers.push(Box::new(ioapic::Discovery {}));
}
