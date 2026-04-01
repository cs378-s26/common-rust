use crate::arch::{Arch, ArchTrait};
#[cfg(target_arch = "aarch64")]
use crate::devices::char::uart_pl011::UartPl011Discovery;
use crate::devices::virtio_discovery::VirtioDiscovery;
use crate::devices::{block::BlockDevice, char::CharDevice, network::NetworkDevice};
use crate::sync::IntMutex;
use crate::sync::MutexLike;
use alloc::{boxed::Box, vec::Vec};
use core::marker::{Send, Sync};
#[cfg(target_arch = "aarch64")]
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
    #[cfg(target_arch = "aarch64")]
    DTB(FdtNode<'a, 'b>),
    #[cfg(target_arch = "x86_64")]
    Acpi(AcpiDeviceNode),
    // suppress unused lifetime warnings when only one variant is active
    #[cfg(not(target_arch = "aarch64"))]
    _Phantom(core::marker::PhantomData<(&'a (), &'b ())>),
}

/// Identifies a device discovered via ACPI table parsing.
#[cfg(target_arch = "x86_64")]
#[derive(Clone, Copy)]
pub enum AcpiDeviceNode {
    Serial16550 { port: u16 },
    IoApic { id: u8, address: u32, gsi_base: u32 },
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
    #[cfg(target_arch = "aarch64")]
    drivers.push(Box::new(UartPl011Discovery));
    drivers.push(Box::new(VirtioDiscovery));
    Arch::create_arch_specific_drivers(&mut drivers);
}
