use alloc::{boxed::Box, vec::Vec};
use fdt::node::FdtNode;
use crate::sync::IntMutex;
use core::marker::{Send, Sync};

// all matched drivers would get pushed to this, and any needed driver would simply have init called on it
pub static MATCHED_DEVICES: IntMutex<Vec<Box<dyn DeviceDriver + Send + Sync>>> = IntMutex::new(Vec::new());

// all implemented drivers in the system, this is what is iterated over to find matches
pub static SYSTEM_DRIVERS: IntMutex<Vec<Box<dyn DeviceDiscovery + Send + Sync>>> = IntMutex::new(Vec::new());


/* ideally what we'd do is define a common DeviceNode struct or trait that gives all the information
necessary for init, including compatibility, interrupts, memory location, etc, but this isn't that easy, for example
compatibility is checked through a compatible string when parsing device tree, but a hardware ID on acpi, 
which I don't think is very easily translatable. What you could do is have  
maybe a struct that has enums for its fields, like an id enum that depends on where it came from, but
it seems difficult to properly specify in a shared struct all information necessary in a clean way. 
*/
pub enum DeviceNode<'a, 'b> { // the idea with this would just be to find what type of node it is, and the driver has to be able to read the fields from the node that it needs
    DTB(FdtNode<'a, 'b>),
    // ACPI(AcpiNode) idk what struct would this be
} // I didn't include pci here because I assumed since it's dynamic it could be done seperately, 
// and probably doesn't need to be tied to a specific arch (I assume?) 

pub enum DeviceType {
    BLOCK,
    CHAR,
    NETWORK,
    OTHER
}

// every driver should implement the following two traits to read device tree nodes and return a match
// the reason for seperating this into two traits is because a driver could match multiple devices, then
// have a different impl based on different device nodes found
pub trait DeviceDriver {

    // defined by the driver, like uart_pl011 or virtio_blk
    fn name(&self) -> &str;

    // this takes no parameters to make it easy to call later, for example a file system can just find
    // the first block device driver and call init without needing to know where the node is. am_i_this should
    // store necessary information for init in the struct. Returns true if succeeded. 
    fn init(&mut self) -> bool;

    fn device_type(&self) -> DeviceType;
    
}

pub trait DeviceDiscovery {

    // when finding a matching node, return a corresponding device driver with its proper fields initialized.
    fn am_i_this(&self, node: DeviceNode) -> Option<Box<dyn DeviceDriver + Send + Sync>>;

}


