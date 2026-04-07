pub mod acpi;
pub mod device_tree;
pub mod pcie;

use crate::arch::{Arch, ArchTrait};
use crate::devices::char::uart_pl011::UartPl011Discovery;
use crate::devices::virtio::discovery::VirtioDiscovery;
use crate::devices::{block::BlockDevice, char::CharDevice, network::NetworkDevice};
use crate::sync::IntMutex;
use crate::sync::MutexLike;
use alloc::{boxed::Box, vec::Vec};
use core::marker::{Send, Sync};
use fdt::node::FdtNode;
use spin::lazy::Lazy;

// lists of initialized devices in the system
pub static BLOCK_DEVICES: IntMutex<Vec<Box<dyn BlockDevice + Send + Sync>>> =
    IntMutex::new(Vec::new());

pub static CHAR_DEVICES: IntMutex<Vec<Box<dyn CharDevice + Send + Sync>>> =
    IntMutex::new(Vec::new());

pub static NETWORK_DEVICES: IntMutex<Vec<Box<dyn NetworkDevice + Send + Sync>>> =
    IntMutex::new(Vec::new());

/// all implemented discovery drivers in the system.
/// DTB discovery walks this list in order and lets the first matching driver
/// claim a node; other walkers may define different stop semantics.
/// maybe make this a read write lock?
pub static SYSTEM_DRIVERS: Lazy<Vec<Box<dyn DeviceDiscovery + Send + Sync>>> =
    Lazy::new(create_drivers);

// Each architecture provides its own variant of DeviceNode. Drivers only match the variant
// for their architecture, so arch-specific types never leak into cross-arch compilation units.
pub enum DeviceNode<'a, 'b> {
    DTB(FdtNode<'a, 'b>),
    MadtEntry(acpi::MadtEntry),
    Pcie(pcie::PcieFunction), // PCIE controller + function number
}

pub enum DeviceType {
    Block(Box<dyn BlockDevice + Send + Sync>),
    Char(Box<dyn CharDevice + Send + Sync>),
    Network(Box<dyn NetworkDevice + Send + Sync>),
    Special, // these are special drivers that interop directly with the system and don't return anything.
}

pub trait DeviceDiscovery {
    // When a node matches, return all device handles it should contribute.
    fn am_i_this(&self, node: DeviceNode) -> Option<Vec<DeviceType>>;

    fn name(&self) -> &'static str;
}

pub fn create_drivers() -> Vec<Box<dyn DeviceDiscovery + Send + Sync>> {
    let mut drivers: Vec<Box<dyn DeviceDiscovery + Send + Sync>> = Vec::new();
    drivers.push(Box::new(UartPl011Discovery));
    drivers.push(Box::new(VirtioDiscovery));
    Arch::create_arch_specific_drivers(&mut drivers);
    drivers
}

pub fn discover_devices() {
    let mut devices = Vec::new();
    if let Some(acpi_devices) = acpi::parse_acpi() {
        devices.extend(acpi_devices);
    }
    if let Some(dtb_devices) = device_tree::parse_device_tree() {
        devices.extend(dtb_devices);
    }

    for device in devices {
        match device {
            DeviceType::Block(d) => BLOCK_DEVICES.lock().push(d),
            DeviceType::Char(d) => CHAR_DEVICES.lock().push(d),
            DeviceType::Network(d) => NETWORK_DEVICES.lock().push(d),
            DeviceType::Special => {}
        }
    }
}
