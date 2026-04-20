use alloc::{boxed::Box, vec::Vec};

use crate::devices::discovery::DeviceDiscovery;

pub fn create_arch_specific_drivers(_: &mut Vec<Box<dyn DeviceDiscovery + Send + Sync>>) {}
