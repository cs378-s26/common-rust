use alloc::vec::Vec;

use spin::once::Once;

use crate::devices::discovery::{DeviceNode, DeviceType, SYSTEM_DRIVERS, acpi::Mcfg, DeviceDiscovery};
use crate::memory::dma::MmioRegion;
use crate::print::kprintln;

pub static PCIE: Once<Pcie> = Once::new();


// a single pcie root
pub struct Pcie {
    pub base_address: u64,
    pub start_bus_number: u8,
    pub end_bus_number: u8,
}

pub struct PcieFunction {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
}

pub struct PcieDiscovery;

impl DeviceDiscovery for PcieDiscovery {
    fn am_i_this(&self, node: DeviceNode) -> Option<Vec<DeviceType>> {
        if let DeviceNode::DTB(fdt_node) = node &&
            let Some(c) = fdt_node.compatible() &&
            c.all().any(|s| s.contains("pci-host-ecam-generic")) {
                // only initialize it one time, per our current assumptions about the system having one pcie root, see below todo
                if PCIE.get().is_none() {
                    let reg = fdt_node.reg().and_then(|mut r| r.next())?;
                    let base_address = reg.starting_address;
                    let size = reg.size.unwrap(); // should always be given for pci-host-ecam-generic node
                    let region = MmioRegion::new(base_address as usize, size as usize); // create a temporary mapping to ensure the region is valid, we will create our own permanent mappings later
                    let (start_bus, end_bus) = match fdt_node.property("bus-range") {
                        Some(prop) => parse_bus_range(prop.value)?,
                        None => (0, 255),
                    };
                    PCIE.call_once(|| Pcie::new(region.virt_addr() as u64, start_bus, end_bus));
                    core::mem::forget(region);
                    kprintln!("Initialized PCIe");
                    return Some(PCIE.get().unwrap().discover());
                }
            }
        None
    }

    fn name(&self) -> &'static str {
        "PCIe"
     }
}

fn parse_bus_range(prop: &[u8]) -> Option<(u8, u8)> {
    if prop.len() != 8 {
        return None;
    }

    let start = u32::from_be_bytes(prop[0..4].try_into().ok()?);
    let end   = u32::from_be_bytes(prop[4..8].try_into().ok()?);

    if start > 255 || end > 255 || start > end {
        return None;
    }

    Some((start as u8, end as u8))
}

impl Pcie {
    pub fn new(base_address: u64, start_bus_number: u8, end_bus_number: u8) -> Self {
        Self { base_address, start_bus_number, end_bus_number }
    }

    pub fn discover(&self) -> Vec<DeviceType> {
        let mut matched_devices = Vec::new();
        for bus in 0..=255 {
            for device in 0..32 {
                for function in 0..8 {
                    if let Some(register_0) = self.read_config_space(bus, device, function, 0x00) {
                        let vendor_id = (register_0 & 0xFFFF) as u16;
                        if vendor_id != 0xFFFF {
                            for driver in SYSTEM_DRIVERS.iter() {
                                let matched_device =
                                    driver.am_i_this(DeviceNode::Pcie(PcieFunction {
                                        bus,
                                        device,
                                        function,
                                    }));
                                if let Some(devices) = matched_device {
                                    matched_devices.extend(devices);
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
        matched_devices
    }

    pub fn read_config_space(&self, bus: u8, device: u8, function: u8, offset: u16) -> Option<u32> {
            if bus >= self.start_bus_number && bus <= self.end_bus_number {
                let relative_bus = bus - self.start_bus_number;
                let address = self.base_address
                    + ((relative_bus as u64) << 20)
                    + ((device as u64) << 15)
                    + ((function as u64) << 12)
                    + (offset as u64);
                return Some(unsafe { core::ptr::read_volatile(address as *const u32) });
            }
        None
    }

    pub fn write_config_space(
        &self,
        bus: u8,
        device: u8,
        function: u8,
        offset: u16,
        value: u32,
    ) -> Option<()> {
            if bus >= self.start_bus_number && bus <= self.end_bus_number {
                let relative_bus = bus - self.start_bus_number;
                let address = self.base_address
                    + ((relative_bus as u64) << 20)
                    + ((device as u64) << 15)
                    + ((function as u64) << 12)
                    + (offset as u64);
                unsafe { core::ptr::write_volatile(address as *mut u32, value) };
                return Some(());
        }
        None
    }
}

pub fn init_pcie_from_mcfg(mcfg: Mcfg) -> Vec<DeviceType> {

    // TODO for now we are just assuming there is one pcie root, we'll need proper identification and bookkeeping to handle multiple,
    // but not top priority for now
    if let Some(entry) = mcfg.iterate_entries().next() {
        let pcie = Pcie::new(entry.base_address, entry.start_bus_number, entry.end_bus_number);
        PCIE.call_once(|| pcie);
        PCIE.get().unwrap().discover()
    } else {
        Vec::new()
    }
}


