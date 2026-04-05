use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::devices::discovery::DeviceDiscovery;

pub fn create_arch_specific_drivers(_: &mut Vec<Box<dyn DeviceDiscovery + Send + Sync>>) {
    // empty for now
}
