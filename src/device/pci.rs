use acpi::{
    sdt::mcfg::Mcfg,
};
use spin::Once;
use x86::io::{inl, outl};
use crate::print::kprintln;
use crate::device::acpi::get_acpi;

const MAX_RESOURCES: usize = 8;
const MAX_CHILDREN: usize = 64;

static DEVICE_ROOT: Once<DeviceNode> = Once::new();

pub enum BusType {
    Platform,
    PCI
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

pub struct DeviceNode {
    pub name: &'static str,
    pub bus: BusType,
    pub parent: Option<&'static DeviceNode>,

    // PCI fields.
    pub vendor_id: u16,
    pub device_id: u16,
    pub class_code: u8,
    pub subclass: u8,
    pub prog_if: u8,
    pub revision: u8,
    pub header_type: u8,
    pub segment: u16,
    pub pci_bus: u8,
    pub pci_dev: u8,
    pub pci_func: u8,
    pub irq_pin: u8,
    pub irq_line: u8,
    pub has_msi: bool,
    pub has_msix: bool,

    // PCI-to-PCI bridge fields.
    pub secondary_bus: u8,
    pub subordinate_bus: u8,

    pub resources: [Resource; MAX_RESOURCES],
    pub resource_count: u32,

    pub children: [Option<&'static DeviceNode>; MAX_CHILDREN],
    pub child_count: u32,

    pub driver_data: Option<&'static ()>,
}

impl DeviceNode {
    pub fn new(name: &'static str, bus: BusType, parent: Option<&'static DeviceNode>) -> Self {
        Self {
            name,
            bus,
            parent,
            vendor_id: 0,
            device_id: 0,
            class_code: 0,
            subclass: 0,
            prog_if: 0,
            revision: 0,
            header_type: 0,
            segment: 0,
            pci_bus: 0,
            pci_dev: 0,
            pci_func: 0,
            irq_pin: 0,
            irq_line: 0,
            has_msi: false,
            has_msix: false,
            secondary_bus: 0,
            subordinate_bus: 0,
            resources: [Resource { resource_type: ResourceType::MMIO, base: 0, length: 0 }; MAX_RESOURCES],
            resource_count: 0,
            children: [None; MAX_CHILDREN],
            child_count: 0,
            driver_data: None,
        }
    }
}

// Header Type 0x0.
#[repr(C, packed)]
struct Device0 {
    pub device_id: u16,
    pub vendor_id: u16,
    pub class_code: u8,
    pub subclass: u8,
    pub prog_if: u8,
    pub revision: u8,
    pub bist: u8,
    pub header_type: u8,
    pub latency_timer: u8,
    pub cache_line_size: u8,
    pub bar: [u32; 6], // Base address 0 to 5.
    pub cardbus_cis_pointer: u32,
    pub subsystem_id: u16,
    pub subsystem_vendor_id: u16,
    pub expansion_rom_base_address: u32,
    reserved0: [u8; 3],
    pub capabilities_pointer: u8,
    reserved1: u32,
    pub max_latency: u8,
    pub min_grant: u8,
    pub interrupt_pin: u8,
    pub interrupt_line: u8,
}

// Header Type 0x1 (PCI-to-PCI bridge).
#[repr(C, packed)]
struct Device1 {
    pub device_id: u16,
    pub vendor_id: u16,
    pub status: u16,
    pub command: u16,
    pub class_code: u8,
    pub subclass: u8,
    pub prog_if: u8,
    pub revision: u8,
    pub bist: u8,
    pub header_type: u8,
    pub latency_timer: u8,
    pub cache_line_size: u8,
    pub bar: [u32; 2], // Base address 0 to 1.
    pub secondary_latency_timer: u8,
    pub subordinate_bus_number: u8,
    pub secondary_bus_number: u8,
    pub primary_bus_number: u8,
    pub secondary_status: u16,
    pub io_limit: u8,
    pub io_base: u8,
    pub memory_limit: u16,
    pub memory_base: u16,
    pub prefetchable_memory_limit: u16,
    pub prefetchable_memory_base: u16,
    pub prefetchable_memory_base_upper: u32, // Upper 32 bits.
    pub prefetchable_memory_limit_upper: u32, // Upper 32 bits.
    pub io_limit_upper: u16, // Upper 16 bits.
    pub io_base_upper: u16, // Upper 16 bits.
    reserved: [u8; 3],
    pub capabilities_pointer: u8,
    pub expansion_rom_base_address: u32,
    pub bridge_control: u16,
    pub interrupt_pin: u8,
    pub interrupt_line: u8,
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
    pub fn read_u8(bus: u8, device: u8, function: u8, offset: u8) -> u8 {
        let address = pci_address(bus, device, function, offset);
        unsafe {
            outl(0xCF8, address);
            let value = inl(0xCFC);
            ((value >> ((offset & 3) * 8)) & 0xFF) as u8
        }
    }

    pub fn read_u16(bus: u8, device: u8, function: u8, offset: u8) -> u16 {
        let address = pci_address(bus, device, function, offset);
        unsafe {
            outl(0xCF8, address);
            let value = inl(0xCFC);
            ((value >> ((offset & 2) * 8)) & 0xFFFF) as u16
        }
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
            // TODO: this doesn't look very thread safe...
            outl(0xCF8, address);
            let mut value = inl(0xCFC);
            value = (value & !(0xFF << ((offset & 3) * 8)))
                | ((value as u32) << ((offset & 3) * 8));
            outl(0xCF8, address); // In case someone changed.
            outl(0xCFC, value);
        }
    }

    pub fn write_u16(bus: u8, device: u8, function: u8, offset: u8, value: u16) {
        let address = pci_address(bus, device, function, offset);
        unsafe {
            outl(0xCF8, address);
            let mut value = inl(0xCFC);
            value = (value & !(0xFFFF << ((offset & 2) * 8)))
                | ((value as u32) << ((offset & 2) * 8));
            outl(0xCF8, address);
            outl(0xCFC, value);
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

fn pci_scan_bus(bus: u8, parent: Option<&'static DeviceNode>) {
    for device in 0..=31 {
        for function in 0..=7 {
            let header = Pci::read_u32(bus, device, function, 0);
            let device_id = (header >> 16) as u16;
            let vendor_id = (header & 0xFFFF) as u16;
            if vendor_id == 0xFFFF {
                continue; // No device.
            }
            kprintln!("Found PCI device: bus={}, device={}, function={}, vendor_id={:#x}, device_id={:#x}",
                bus, device, function, vendor_id, device_id);
        }
    }
}

pub fn init_pci() {
    kprintln!("Initializing PCI.");
    let root_node = DEVICE_ROOT.call_once(|| DeviceNode::new("system", BusType::Platform, None));

    let acpi_info = get_acpi();
    if let Some(mcfg) = acpi_info.tables.find_table::<Mcfg>() {
        for entry in mcfg.entries() {
            let base_address = entry.base_address;
            let segment_group = entry.pci_segment_group;
            let bus_start = entry.bus_number_start;
            let bus_end = entry.bus_number_end;

            kprintln!("PCI: base={:#x}, segment_group={:#x}, bus_start={:#x}, bus_end={:#x}",
                base_address, segment_group, bus_start, bus_end);
        }
        // TODO: MCFG stuff.
    } else {
        kprintln!("PCI: MCFG table not found");
        // Legacy PCI scanning.
        for bus in 0..=255 {
            pci_scan_bus(bus, Some(root_node));
        }
    }
    kprintln!("PCI initialized.");
}

