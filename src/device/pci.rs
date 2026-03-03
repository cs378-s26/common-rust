use crate::device::acpi::get_acpi;
use crate::print::kprintln;
use acpi::platform::pci;
use acpi::sdt::mcfg::Mcfg;
use crate::sync::IntMutex;
use alloc::boxed::Box;
use alloc::sync::Arc;
use spin::Once;
use x86::io::{inl, outl};

const MAX_RESOURCES: usize = 8;
const MAX_CHILDREN: usize = 64;

static DEVICE_ROOT: Once<Arc<dyn Device + Send + Sync>> = Once::new();

const INVALID_VENDOR_ID: u16 = 0xFFFF;
const HEADER_TYPE_OFFSET: u8 = 0xD;
const SECONDARY_BUS_OFFSET: u8 = 0x1A;
// If more fields needed: https://wiki.osdev.org/PCI

pub enum BusType {
    Platform,
    PCI,
}

#[derive(Copy, Clone)]
pub enum ResourceType {
    MMIO,
    IOPort,
    IRQ,
}

#[derive(Copy, Clone)]
pub struct Resource {
    pub resource_type: ResourceType,
    pub base: u64,
    pub length: u64,
}

//TODO: abstract PCIDeviceNode into a trait
//  Trait will require intrusive storage of all PCI Metadata
//  Have a PCI Bridge trait, only PCI bridges will have children
//  Have a function in device trait, as_bridge -> Option<dyn Bridge>

pub trait Bridge {
    //TODO: replace with RwLock
    fn children() -> IntMutex<Vec<Arc<dyn Device>>>;
}

pub struct DeviceNode {
    pub address: u32,
    pub name: &'static str,
    pub bus: BusType,
    //pub parent: *mut DeviceNode, // TODO: do we want to free devices ever? Someone said not to use Arc/Box.

    // Common header fields.
    pub device_id: u16,
    pub vendor_id: u16,
    pub class_code: u8,
    pub subclass: u8,
    pub prog_if: u8,
    pub revision_id: u8,
    pub header_type: u8,

    pub resources: [Resource; MAX_RESOURCES],
    pub resource_count: usize,

    pub children: [Option<Arc<DeviceNode>>; MAX_CHILDREN],
    pub child_count: usize,

    pub driver: Option<()>, // TODO.
}

// TODO: figure out how to make this safe. D:
// This seems terribly unnecessary...
unsafe impl Send for DeviceNode {}
unsafe impl Sync for DeviceNode {}

// 

impl DeviceNode {
    pub fn new(address: u32, name: &'static str, bus: BusType) -> Self {
        Self {
            address,
            name,
            bus,
            device_id: 0,
            vendor_id: 0,
            class_code: 0,
            subclass: 0,
            prog_if: 0,
            revision_id: 0,
            header_type: 0,
            resources: [Resource {
                resource_type: ResourceType::MMIO,
                base: 0,
                length: 0,
            }; MAX_RESOURCES],
            resource_count: 0,
            children: [const {None}; MAX_CHILDREN],
            child_count: 0,
            driver: None,
        }
    }

    pub fn add_child(&mut self, child : DeviceNode) {
        if self.child_count < MAX_CHILDREN {
            //let child = DeviceNode::new(address, name, bus);
            let child_ptr = Arc::new(child);
            self.children[self.child_count] = Some(child_ptr.clone());
            self.child_count += 1;
            //child_ptr
        } else {
            panic!(
                "Maximum number of children reached for device {}.",
                self.name
            );
        }
    }

    pub fn add_resource(&mut self, resource_type: ResourceType, base: u64, length: u64) {
        if self.resource_count < MAX_RESOURCES {
            self.resources[self.resource_count] = Resource {
                resource_type,
                base,
                length,
            };
            self.resource_count += 1;
        } else {
            panic!(
                "Maximum number of resources reached for device {}.",
                self.name
            );
        }
    }
}

// TODO: do we want PCI-to-CardBus?

// Legacy version.
fn pci_address(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    1 << 31
        | (bus as u32) << 16
        | (device as u32) << 11
        | (function as u32) << 8
        | (offset as u32 & 0xfc)
}

pub struct Pci;
impl Pci {
    // TODO: this all doesn't look very thread safe... Turn off preemption or interrupts.
    pub fn read_u8(bus: u8, device: u8, function: u8, offset: u8) -> u8 {
        ((Self::read_u32(bus, device, function, offset) >> ((offset & 3) * 8)) & 0xFF) as u8
    }

    pub fn read_u16(bus: u8, device: u8, function: u8, offset: u8) -> u16 {
        ((Self::read_u32(bus, device, function, offset) >> ((offset & 2) * 8)) & 0xFFFF) as u16
    }

    pub fn read_u32(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
        let address = pci_address(bus, device, function, offset);
        unsafe {
            outl(0xCF8, address);
            inl(0xCFC)
        }
    }

    pub fn write_u8(bus: u8, device: u8, function: u8, offset: u8, value: u8) {
        let address = pci_address(bus, device, function, offset);
        unsafe {
            outl(0xCF8, address);
            let mut old_value = inl(0xCFC);
            old_value &= !(0xFF << ((offset & 3) * 8));
            old_value |= (value as u32) << ((offset & 3) * 8);
            outl(0xCFC, old_value);
        }
    }

    pub fn write_u16(bus: u8, device: u8, function: u8, offset: u8, value: u16) {
        let address = pci_address(bus, device, function, offset);
        unsafe {
            outl(0xCF8, address);
            let mut old_value = inl(0xCFC);
            old_value &= !(0xFFFF << ((offset & 2) * 8));
            old_value |= (value as u32) << ((offset & 2) * 8);
            outl(0xCFC, old_value);
        }
    }

    pub fn write_u32(bus: u8, device: u8, function: u8, offset: u8, value: u32) {
        let address = pci_address(bus, device, function, offset);
        unsafe {
            outl(0xCF8, address);
            outl(0xCFC, value);
        }
    }
}

struct GenericBridge {
    pub device_id: u16,
    pub vendor_id: u16,
    //we know what these are gonna be
    //pub class_code: u8,
    //pub subclass: u8,
    pub address: u32,
    pub children: IntMutex<Vec<Arc<dyn Device>>>
    //TODO: look at bridge
}
use alloc::vec::Vec;
use alloc::vec;
impl GenericBridge {
    pub fn new(bus_num: u8, address: u32, device_id: u16, vendor_id: u16) -> GenericBridge {
        let children : Vec<Arc<dyn Device>> = pci_scan_bus(bus_num);
        Self {
            device_id,
            vendor_id,
            address,
            children : IntMutex::new(children)
        }
    }
}

struct GenericDevice {
    pub device_id: u16,
    pub vendor_id: u16,
    pub address: u32, 
    pub class_code: u8,
    pub subclass: u8
}

struct HostController {
    pub address: u32, 
    pub children: IntMutex<Vec<Arc<dyn Device>>>
}

struct PCIRoot {
    pub children: IntMutex<Vec<Arc<dyn Device>>>
}
pub trait Device {
    fn device_id(&self) -> u16;
    fn vendor_id(&self) -> u16;
    fn name(&self) -> &'static str;
    fn children(&self) -> Option<&IntMutex<Vec<Arc<dyn Device>>>>; // Only for bridges.
}

impl Device for GenericBridge {
        fn device_id(&self) -> u16 {
            self.device_id
        }
    
        fn vendor_id(&self) -> u16 {
            self.vendor_id
        }
    
        fn name(&self) -> &'static str {
            "Generic PCI Bridge"
        }
    
        fn children(&self) -> Option<&IntMutex<Vec<Arc<dyn Device>>>> {
            Some(&self.children)
        }
}

impl Device for GenericDevice {
    fn device_id(&self) -> u16 {
        self.device_id
    }

    fn vendor_id(&self) -> u16 {
        self.vendor_id
    }

    fn name(&self) -> &'static str {
        "Generic PCI Device"
    }

    fn children(&self) -> Option<&IntMutex<Vec<Arc<dyn Device>>>> {
        None
    }
}

impl Device for HostController {
    fn device_id(&self) -> u16 {
        0
    }

    fn vendor_id(&self) -> u16 {
        0
    }

    fn name(&self) -> &'static str {
        "PCI Host Controller"
    }

    fn children(&self) -> Option<&IntMutex<Vec<Arc<dyn Device>>>> {
        Some(&self.children)
    }
}

impl Device for PCIRoot {
    fn device_id(&self) -> u16 {
        0
    }

    fn vendor_id(&self) -> u16 {
        0
    }

    fn name(&self) -> &'static str {
        "PCI Root"
    }

    fn children(&self) -> Option<&IntMutex<Vec<Arc<dyn Device>>>> {
        Some(&self.children)
    }
}

pub fn traverse_tree(device: &Arc<dyn Device>, depth: usize) {
    let indent = "  ".repeat(depth);
    kprintln!("{}Device: {}, vendor_id={:#x}, device_id={:#x}", indent, device.name(), device.vendor_id(), device.device_id());
    if let Some(children) = device.children() {
        for child in children.lock().iter() {
            traverse_tree(child, depth + 1);
        }
    } 
}


//TODO: figure out how we're going to interact with drivers
fn pci_scan_function(bus: u8, device: u8, function: u8, register0: u32) -> 
    Option<Arc<dyn Device>> {
    let device_id = (register0 >> 16) as u16;
    let vendor_id = (register0 & 0xFFFF) as u16;
    if vendor_id == INVALID_VENDOR_ID {
        None
    } else {
        kprintln!(
            "Found PCI device: bus={}, device={}, function={}, vendor_id={:#x}, device_id={:#x}",
            bus,
            device,
            function,
            vendor_id,
            device_id
        );

        let register1 = Pci::read_u32(bus, device, function, 0x4);
        let register2 = Pci::read_u32(bus, device, function, 0x8);
        let register3 = Pci::read_u32(bus, device, function, 0xC);

        let status = (register1 >> 16) as u16;
        let command = (register1 & 0xFFFF) as u16;
        let class_code = (register2 >> 24) as u8;
        let subclass = ((register2 >> 16) & 0xFF) as u8;
        let prog_if = ((register2 >> 8) & 0xFF) as u8;
        let revision_id = (register2 & 0xFF) as u8;
        let header_type = ((register3 >> 16) & 0xFF) as u8;
        kprintln!(
            "  status={:#x}, command={:#x}, class_code={:#x}, subclass={:#x}, prog_if={:#x}, revision_id={:#x}, header_type={:#x}",
            status,
            command,
            class_code,
            subclass,
            prog_if,
            revision_id,
            header_type
        );

        if class_code == 0x06 && subclass == 0x04 {
            let secondary_bus = Pci::read_u8(bus, device, function, SECONDARY_BUS_OFFSET);
            Some(Arc::new(GenericBridge::new(
                secondary_bus, 
                pci_address(bus, device, function, 0),
                device_id,
                vendor_id
            )))
        } else {
            Some(Arc::new(GenericDevice{
                device_id,
                vendor_id,
                address: pci_address(bus, device, function, 0),
                class_code,
                subclass
            }))
        }
    }
}   

fn pci_scan_bus(bus: u8) -> Vec<Arc<dyn Device>> {
    let mut devices : Vec<Arc<dyn Device>> = vec![];
    for device in 0..=31 {
        let register0 = Pci::read_u32(bus, device, 0, 0);
        if (register0 & 0xFFFF) as u16 == INVALID_VENDOR_ID {
            continue; // No device.
        }
        if let Some(dev) = pci_scan_function(bus, device, 0, register0) {
            devices.push(dev);
        }

        let header_type = Pci::read_u8(bus, device, 0, HEADER_TYPE_OFFSET);
        if header_type & 0x80 != 0 {
            // Multifunction.
            for function in 1..=7 {
                let register0 = Pci::read_u32(bus, device, function, 0);
                if (register0 & 0xFFFF) as u16 == INVALID_VENDOR_ID {
                    continue; // No device.
                }
                if let Some(dev) = pci_scan_function(bus, device, function, register0) {
                    devices.push(dev);
                }
            }
        }
    }
    devices
}

pub fn init_pci() {
    kprintln!("Initializing PCI.");
    //let root = DeviceNode::new(0, "root", BusType::PCI);
    let acpi_info = get_acpi();
    if let Some(mcfg) = acpi_info.tables.find_table::<Mcfg>() {
        for entry in mcfg.entries() {
            let base_address = entry.base_address;
            let segment_group = entry.pci_segment_group;
            let bus_start = entry.bus_number_start;
            let bus_end = entry.bus_number_end;

            kprintln!(
                "PCI: base={:#x}, segment_group={:#x}, bus_start={:#x}, bus_end={:#x}",
                base_address,
                segment_group,
                bus_start,
                bus_end
            );
        }
        // TODO: MCFG stuff.
    } else {
        kprintln!("PCI: MCFG table not found");
        // Legacy PCI scanning.
        // TODO: this also seems terribly unnecessary.
        let first_header_type = Pci::read_u8(0, 0, 0, HEADER_TYPE_OFFSET);
        if first_header_type & 0x80 == 0 {
            // Single PCI host controller.
            //let mut pci_host = root_ptr.add_child(0, "PCI Host Controller", BusType::PCI);
            let children = pci_scan_bus(0);
            let pci_host : Arc<dyn Device>  = Arc::new(
                HostController {
                    address: pci_address(0, 0, 0, 0),
                    children: IntMutex::new(children)
                }
            );
            traverse_tree(&pci_host, 0);
        } else {
            // this was some surprisingly elegant code by our little friend
            let controllers : Vec<Arc<dyn Device>> = (0..=7).filter_map(|function| {
                let register0 = Pci::read_u32(0, 0, function, 0);
                if (register0 & 0xFFFF) as u16 == INVALID_VENDOR_ID {
                    None
                } else {
                    let d : Arc<dyn Device> = Arc::new(
                        HostController {
                            address: pci_address(0, 0, function, 0),
                            children: IntMutex::new(pci_scan_bus(function))
                        }
                    );
                    Some(d)
                }
            }).collect();
            let root : Arc<dyn Device> = Arc::new(
                PCIRoot {
                    children: IntMutex::new(controllers)
                }
            );
            traverse_tree(&root, 0);
        }
    }
    kprintln!("PCI initialized.");
}
