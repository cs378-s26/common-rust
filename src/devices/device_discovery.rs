use crate::devices::{block::BlockDevice, char::CharDevice};
use crate::sync::IntMutex;
use alloc::{boxed::Box, vec::Vec};
use core::marker::{Send, Sync};
use fdt::node::FdtNode;

// lists of initialized devices in the system
pub static BLOCK_DEVICES: IntMutex<Vec<Box<dyn BlockDevice + Send + Sync>>> =
    IntMutex::new(Vec::new());

pub static CHAR_DEVICES: IntMutex<Vec<Box<dyn CharDevice + Send + Sync>>> =
    IntMutex::new(Vec::new());

// pub static NETWORK_DEVICES: IntMutex<Vec<Box<dyn DeviceDriver + Send + Sync>>> =
//     IntMutex::new(Vec::new());

/// all implemented drivers in the system, this is what is iterated over to find matches.
/// order matters here, the first matched driver will get assigned the device.
pub static SYSTEM_DRIVERS: IntMutex<Vec<Box<dyn DeviceDiscovery + Send + Sync>>> =
    IntMutex::new(Vec::new());

/* ideally what we'd do is define a common DeviceNode struct or trait that gives all the information
necessary for init, including compatibility, interrupts, memory location, etc, but this isn't that easy, for example
compatibility is checked through a compatible string when parsing device tree, but a hardware ID on acpi,
which I don't think is very easily translatable. What you could do is have
maybe a struct that has enums for its fields, like an id enum that depends on where it came from, but
it seems difficult to properly specify in a shared struct all information necessary in a clean way.
*/
pub enum DeviceNode<'a, 'b> {
    // the idea with this would just be to find what type of node it is, and the driver has to be able to read the fields from the node that it needs
    DTB(FdtNode<'a, 'b>),
    // ACPI(AcpiNode) idk what struct would this be
} // I didn't include pci here because I assumed since it's dynamic it could be done seperately,

pub enum DeviceType {
    Block(Box<dyn BlockDevice + Send + Sync>),
    Char(Box<dyn CharDevice + Send + Sync>),
    Special, // these are special drivers that interop directly with the system and don't return anything.
}

pub trait DeviceDiscovery {
    // when finding a matching node, return a corresponding device driver with its proper fields initialized.
    fn am_i_this(&self, node: DeviceNode) -> Option<DeviceType>;
}
