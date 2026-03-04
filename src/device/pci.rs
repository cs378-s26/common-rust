use crate::device::acpi::get_acpi;
use crate::print::kprintln;
use crate::arch::{Arch, ArchTrait};
use crate::virtual_memory::{VirtualMemoryAllocation, PagingOptions};
use crate::physical_memory::HHDM_OFFSET;
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
    pub address: usize,
    pub children: IntMutex<Vec<Arc<dyn Device>>>
    //TODO: look at bridge
}
use alloc::vec::Vec;
use alloc::vec;
impl GenericBridge {
    pub fn new(bus_num: u8, address: usize, device_id: u16, vendor_id: u16) -> GenericBridge {
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

struct GenericPcieDevice {
    pub device_id: u16,
    pub vendor_id: u16,
    pub class_code: u8,
    pub subclass: u8,
    pub cfg: PcieConfigSpace
}

struct GenericPcieBridge {
    pub device_id: u16,
    pub vendor_id: u16,
    pub class_code: u8,
    pub subclass: u8,
    pub cfg: PcieConfigSpace,
    pub children: IntMutex<Vec<Arc<dyn Device>>>
}

impl GenericPcieBridge {
    pub fn new(cfg: PcieConfigSpace, bus_num: u8, device_id: u16, vendor_id: u16) -> GenericPcieBridge {
        let children : Vec<Arc<dyn Device>> = pcie_scan_bus(0, bus_num);
        Self {
            device_id,
            vendor_id,
            class_code: 0x06,
            subclass: 0x04,
            cfg,
            children : IntMutex::new(children)
        }
    }
}

impl Device for GenericPcieBridge {
    fn device_id(&self) -> u16 {
        self.device_id
    }

    fn vendor_id(&self) -> u16 {
        self.vendor_id
    }

    fn name(&self) -> &'static str {
        "Generic PCIe Bridge"
    }

    fn children(&self) -> Option<&IntMutex<Vec<Arc<dyn Device>>>> {
        Some(&self.children)
    }
}

impl Device for GenericPcieDevice {
    fn device_id(&self) -> u16 {
        self.device_id
    }

    fn vendor_id(&self) -> u16 {
        self.vendor_id
    }

    fn name(&self) -> &'static str {
        "Generic PCIe Device"
    }

    fn children(&self) -> Option<&IntMutex<Vec<Arc<dyn Device>>>> {
        None
    }
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
                pci_address(bus, device, function, 0) as usize,
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

struct PcieMMIO {
    base : usize
}

struct PcieConfigSpace {
    bus: u8,
    device: u8,
    function: u8,
    mapping: VirtualMemoryAllocation
}


impl PcieConfigSpace {
    pub fn new(base : usize, bus: u8, device: u8, function: u8) -> Self {
        let address = pcie_address(base, bus, device, function, 0);
        let mapping = 
            VirtualMemoryAllocation::new(Arch::get_address_space(), 
            4096, 
            Some(address), 
            PagingOptions::PRESENT | PagingOptions::WRITABLE | PagingOptions::WRITE_THROUGH);
        Self {
            bus,
            device,
            function,
            mapping
        }
    }
}

impl PcieConfigSpace {
    fn read_u32(&self, offset: u16) -> u32 {
        let address = self.mapping.base + (offset & 0xfff) as usize;
        kprintln!("Reading from PCIe config space at address {:#x}", address);
        unsafe { core::ptr::read_volatile(address as *const u32) }
    }

    fn write_u32(&self, offset: u16, value: u32) {
        let address = self.mapping.base + (offset & 0xfff) as usize;
        unsafe { core::ptr::write_volatile(address as *mut u32, value) }
    }

    fn read_u8(&self, offset: u16) -> u8 {
        ((self.read_u32(offset) >> ((offset & 3) * 8)) & 0xFF) as u8
    }

    fn read_u16(&self, offset: u16) -> u16 {
        ((self.read_u32(offset) >> ((offset & 2) * 8)) & 0xFFFF) as u16
    }
}

trait PciAccess {
    fn read_u8(&self, bus : u8, device : u8, function : u8, offset : u8) -> u8 {
        ((self.read_u32(bus, device, function, offset) >> ((offset & 3) * 8)) & 0xFF) as u8
    }
    fn read_u16(&self, bus : u8, device : u8, function : u8, offset : u8) -> u16 {
        ((self.read_u32(bus, device, function, offset) >> ((offset & 2) * 8)) & 0xFFFF) as u16
    }
    fn read_u32(&self, bus : u8, device : u8, function : u8, offset : u8) -> u32;
    fn write_u8(&self, bus : u8, device : u8, function : u8, offset : u8, value : u8) {
        let current_value = self.read_u32(bus, device, function, offset);
        let new_value = (current_value & !(0xFF << ((offset & 3) * 8))) | ((value as u32) << ((offset & 3) * 8));
        self.write_u32(bus, device, function, offset, new_value);
    }
    fn write_u16(&self, bus : u8, device : u8, function : u8, offset : u8, value : u16) {
        let current_value = self.read_u32(bus, device, function, offset);
        let new_value = (current_value & !(0xFFFF << ((offset & 2) * 8))) | ((value as u32) << ((offset & 2) * 8));
        self.write_u32(bus, device, function, offset, new_value);
    }
    fn write_u32(&self, bus : u8, device : u8, function : u8, offset : u8, value : u32);
}

fn pcie_address(base : usize, bus: u8, device: u8, function: u8, offset: u8) -> usize {
        (base + (((bus as u64) << 20)
        | ((device as u64) << 15)
        | ((function as u64) << 12)
        | (offset as u64 & 0xFFF)) as usize) as usize
}

impl PciAccess for PcieMMIO {
    fn read_u32(&self, bus: u8, device: u8, function: u8, offset: u8) -> u32 {
        let address = pcie_address(self.base, bus, device, function, offset);
        unsafe { core::ptr::read_volatile(address as *const u32) }
    }

    fn write_u32(&self, bus: u8, device: u8, function: u8, offset: u8, value: u32) {
        let address = pcie_address(self.base, bus, device, function, offset) as *mut u32;
        unsafe { core::ptr::write_volatile(address as *mut u32, value) }
    }
}



fn pcie_scan_bus(base : usize, bus: u8) -> Vec<Arc<dyn Device>> {
    let mut devices : Vec<Arc<dyn Device>> = vec![];
    for device in 0..=31 {
        let mapping = PcieConfigSpace::new(base, bus, device, 0);
        let vendor_id = mapping.read_u16(0);
        if (vendor_id & 0xFFFF) as u16 == INVALID_VENDOR_ID {
            continue;
        }
        kprintln!("Found PCIe device: bus={}, device={}, vendor_id={:#x}", bus, device, vendor_id);
        devices.append(&mut pcie_scan_device(mapping, base, bus, device));
        kprintln!("Done scanning device");
        //core::mem::drop(mapping); // this is really just to make sure we don't have too many mappings open at once, since each device has its own config space mapping. We could also just reuse the same mapping for every device, but this is easier for now.
    }
    devices
}

fn pcie_scan_device(mapping: PcieConfigSpace, base : usize, bus : u8, device : u8) -> Vec<Arc<dyn Device>> {
    let mut functions : Vec<Arc<dyn Device>> = vec![];
    let header_type = mapping.read_u32(0xc) >> 16;
    let reg0 = mapping.read_u32(0);
    kprintln!("Mapping address: {:x}", mapping.mapping.base);
    if let Some(dev) = pcie_scan_function(mapping, reg0) {
        kprintln!("Pushing device");
        functions.push(dev);
    }
    if header_type & 0x80 != 0 {
        for function in 1..=7 {
            let mapping = PcieConfigSpace::new(base, bus, device, function);
            let vendor_id = reg0 & 0xFFFF;
            if (vendor_id & 0xFFFF) as u16 == INVALID_VENDOR_ID {
                continue;
            }
            if let Some(dev) = pcie_scan_function(mapping, reg0) {
                kprintln!("Pushing device");
                functions.push(dev);
            }
        }
    }
    functions
}

fn pcie_scan_function(handle: PcieConfigSpace, register0: u32) -> 
    Option<Arc<dyn Device>> {
    let device_id = (register0 >> 16) as u16;
    let vendor_id = (register0 & 0xFFFF) as u16;
    if vendor_id == INVALID_VENDOR_ID {
        None
    } else {
        kprintln!(
            "Found PCI device: vendor_id={:#x}, device_id={:#x}",
            vendor_id,
            device_id
        );

        let register2 = handle.read_u32( 0x8);
        let class_code = (register2 >> 24) as u8;
        let subclass = ((register2 >> 16) & 0xFF) as u8;

        if class_code == 0x06 && subclass == 0x04 {
            kprintln!("Found PCIe bridge: vendor_id={:#x}, device_id={:#x}", vendor_id, device_id);
            let secondary_bus = handle.read_u8(SECONDARY_BUS_OFFSET.into());
            Some(Arc::new(GenericPcieBridge::new(
                handle,
                secondary_bus, 
                device_id,
                vendor_id
            )))
        } else {
            kprintln!("Creating new GenericPCIeDevice for vendor_id={:#x}, device_id={:#x}", vendor_id, device_id);
            let ret: Option<Arc<dyn Device>> = Some(Arc::new(GenericPcieDevice{
                 cfg: handle,
                 device_id,
                 vendor_id,
                 class_code,
                 subclass
            }));
            ret
        }
    }
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
            let bus_end: u8 = entry.bus_number_end;

            kprintln!(
                "PCI: base={:#x}, segment_group={:#x}, bus_start={:#x}, bus_end={:#x}",
                base_address,
                segment_group,
                bus_start,
                bus_end
            );

            let mut children : Vec<Arc<dyn Device>> = vec![];
            for bus in bus_start..=bus_end {
                let mut bus_devices = pcie_scan_bus(base_address as usize, bus);
                children.append(&mut bus_devices);
            }
            let root : Arc<dyn Device> = Arc::new(
                PCIRoot {
                    children: IntMutex::new(children)
                }
            );
            kprintln!("Traversing tree");
            traverse_tree(&root, 0);
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
