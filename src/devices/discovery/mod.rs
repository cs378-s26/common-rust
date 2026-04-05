pub mod acpi;
pub mod device_tree;

use crate::arch::{Arch, ArchTrait};
use crate::devices::char::uart_pl011::UartPl011Discovery;
use crate::devices::virtio_discovery::VirtioDiscovery;
use crate::devices::{block::BlockDevice, char::CharDevice, network::NetworkDevice};
use crate::sync::IntMutex;
use crate::sync::MutexLike;
use ::acpi::aml::object::WrappedObject;
use ::acpi::sdt::{self};
use alloc::{boxed::Box, vec::Vec};
use core::marker::{Send, Sync};
use fdt::node::FdtNode;

// lists of initialized devices in the system
pub static BLOCK_DEVICES: IntMutex<Vec<Box<dyn BlockDevice + Send + Sync>>> =
    IntMutex::new(Vec::new());

pub static CHAR_DEVICES: IntMutex<Vec<Box<dyn CharDevice + Send + Sync>>> =
    IntMutex::new(Vec::new());

pub static NETWORK_DEVICES: IntMutex<Vec<Box<dyn NetworkDevice + Send + Sync>>> =
    IntMutex::new(Vec::new());

/// all implemented drivers in the system, this is what is iterated over to find matches.
/// order matters here, the first matched driver will get assigned the device.
pub static SYSTEM_DRIVERS: IntMutex<Vec<Box<dyn DeviceDiscovery + Send + Sync>>> =
    IntMutex::new(Vec::new());

// Each architecture provides its own variant of DeviceNode. Drivers only match the variant
// for their architecture, so arch-specific types never leak into cross-arch compilation units.
pub enum DeviceNode<'a, 'b> {
    DTB(FdtNode<'a, 'b>),
    Acpi(AcpiDeviceNode<'a>),
}

pub enum AcpiDeviceNode<'a> {
    MadtEntry(sdt::madt::MadtEntry<'a>),
    WrappedObject(WrappedObject),
}

pub enum DeviceType {
    Block(Box<dyn BlockDevice + Send + Sync>),
    Char(Box<dyn CharDevice + Send + Sync>),
    Network(Box<dyn NetworkDevice + Send + Sync>),
    Special, // these are special drivers that interop directly with the system and don't return anything.
}

pub trait DeviceDiscovery {
    // when finding a matching node, return a corresponding device driver with its proper fields initialized.
    fn am_i_this(&self, node: DeviceNode) -> Option<DeviceType>;

    fn name(&self) -> &'static str;
}

pub fn create_drivers() {
    let mut drivers = SYSTEM_DRIVERS.lock();
    drivers.push(Box::new(UartPl011Discovery));
    drivers.push(Box::new(VirtioDiscovery));
    Arch::create_arch_specific_drivers(&mut drivers);
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
