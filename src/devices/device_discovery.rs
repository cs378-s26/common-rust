use alloc::boxed::Box;

pub struct DeviceNode<'a> {
    name: &'a str,
    compatible: &'a str
}

pub enum DeviceType {
    BLOCK,
    CHAR,
    NETWORK,
    OTHER
}

// every driver should implement this trait to read device tree nodes and find a match
pub trait DeviceDiscovery {

    // when finding a matching node, each driver should either call init or return itself
    fn am_i_this(&self, node: DeviceNode) -> Option<Box<dyn DeviceDiscovery>>;

    fn name(&self) -> &str;

    fn init(&self);

    fn device_type(&self) -> DeviceType;
    
}


