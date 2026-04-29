use alloc::{boxed::Box, vec::Vec};

use spin::once::Once;

use crate::{
    Arch, ArchTrait,
    devices::discovery::{DeviceNode, DeviceType, SYSTEM_DRIVERS, acpi::Mcfg},
    memory::dma::MmioRegion,
};

pub static PCIE: Once<Pcie> = Once::new();

pub struct Pcie {
    mcfg: Mcfg,
}

pub struct PcieFunction {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
}

impl PcieFunction {
    fn new(bus: u8, device: u8, function: u8) -> Self {
        Self {
            bus,
            device,
            function,
        }
    }

    pub fn read_config_space(&self, offset: u16) -> Option<u32> {
        PCIE.get()
            .unwrap()
            .read_config_space(self.bus, self.device, self.function, offset)
    }

    pub fn write_config_space(&self, offset: u16, value: u32) -> Option<()> {
        PCIE.get()
            .unwrap()
            .write_config_space(self.bus, self.device, self.function, offset, value)
    }
}

pub fn enable_bus_mastering(pcie_func: &mut PcieFunction) -> Option<()> {
    let command_reg = pcie_func.read_config_space(0x04)?;
    pcie_func.write_config_space(0x04, command_reg | 0x4)
}

pub fn enable_interrupts(pcie_func: &mut PcieFunction) -> Option<()> {
    let command_reg = pcie_func.read_config_space(0x04)?;
    pcie_func.write_config_space(0x04, command_reg | 0x1)
}

pub fn enable_mem_space(pcie_func: &mut PcieFunction) -> Option<()> {
    let command_reg = pcie_func.read_config_space(0x04)?;
    pcie_func.write_config_space(0x04, command_reg | 0x2)
}

/*
* Maps `bar_num` BAR to an MmioRegion. Returns None if the BAR is an I/O BAR, or if there was an error reading the config space.
* Do not try to map the same bar of the same configuration space twice.
*/
pub fn map_bar(pcie_func: &mut PcieFunction, bar_num: u16) -> Option<MmioRegion> {
    let cur_bar = pcie_func.read_config_space(0x10 + bar_num * 4).unwrap();
    let is_mmio = cur_bar & 1 == 0;
    if is_mmio {
        let type_ = (cur_bar >> 1) & 0b11;
        if type_ == 2 {
            //64-bit address, so we take from the next BAR too
            let next_bar = pcie_func
                .read_config_space(0x10 + (bar_num + 1) * 4)
                .unwrap();
            let bar_addr = (cur_bar & 0xFFFFFFF0) as u64 | ((next_bar as u64) << 32);
            //get size
            pcie_func.write_config_space(0x10 + bar_num * 4, 0xFFFF_FFFF);
            pcie_func.write_config_space(0x10 + (bar_num + 1) * 4, 0xFFFF_FFFF);
            let sz0 =
                (pcie_func.read_config_space(0x10 + bar_num * 4).unwrap() & 0xFFFFFFF0) as u64;
            let sz1 = pcie_func
                .read_config_space(0x10 + (bar_num + 1) * 4)
                .unwrap() as u64;
            let bar_size: usize = (!(sz0 | (sz1 << 32)) + 1) as usize;
            //restore BARs;
            pcie_func.write_config_space(0x10 + bar_num * 4, cur_bar);
            pcie_func.write_config_space(0x10 + (bar_num + 1) * 4, next_bar);
            let bar_size = bar_size.max(0x10000);
            let bar_region = MmioRegion::new(bar_addr as usize, bar_size);
            Some(bar_region)
        } else {
            //we assume this is a 32-bit region.
            let bar_addr = cur_bar & 0xFFFFFFF0;
            pcie_func.write_config_space(0x10 + bar_num * 4, 0xFFFF_FFFF);
            let sz = (!(pcie_func.read_config_space(0x10 + bar_num * 4).unwrap() & 0xFFFFFFF0) + 1)
                as u64;
            pcie_func.write_config_space(0x10 + bar_num * 4, cur_bar);
            let bar_size = sz.max(0x10000);
            let bar_region = MmioRegion::new(bar_addr as usize, bar_size as usize);
            Some(bar_region)
        }
    } else {
        //i/o space, we ignore for now
        None
    }
}

pub const PCI_CAP_MSI: u8 = 0x05;
pub const PCI_CAP_MSIX: u8 = 0x11;

const PCI_CFG_CAP_PTR: u16 = 0x34;
const MSI_MC_64BIT_BIT: u32 = 1 << 23;
const MSI_MC_ENABLE_BIT: u32 = 1 << 16;
const MSI_MC_MME_MASK: u32 = 0x7 << 20; 

pub fn find_capability(pcie_func: &PcieFunction, cap_id: u8) -> Option<u16> {
    let mut offset = (pcie_func.read_config_space(PCI_CFG_CAP_PTR)? & 0xFC) as u16;
    let mut hops = 0;
    while offset != 0 && hops < 48 {
        let cap = pcie_func.read_config_space(offset)?;
        if (cap & 0xFF) as u8 == cap_id {
            return Some(offset);
        }
        offset = ((cap >> 8) & 0xFC) as u16;
        hops += 1;
    }
    None
}

/*
* Returns (table_bir, cap_offset, table_offset) where
* table_bir is the BAR index where the MSI-X table is located
* cap_offset is the offset in the PCI config space where the MSI-X capability is located
* table_offset is the offset within the BAR where the MSI-X table is located
*/
pub fn get_msix_table(pcie_func: &mut PcieFunction) -> Option<(u8, u16, u32)> {
    let msix_cap = find_capability(pcie_func, PCI_CAP_MSIX)?;
    let table_bir = (pcie_func.read_config_space(msix_cap + 0x4).unwrap() & 0x7) as u8;
    let table_offset = pcie_func.read_config_space(msix_cap + 0x4).unwrap() & !(0x7);
    Some((table_bir, msix_cap, table_offset))
}

pub fn register_msi_handler(
    handle: &mut PcieFunction,
    msi_cap_off: u16,
    irq_vec: Option<u8>,
    handler: Box<dyn (Fn() -> Option<()>) + Send + Sync>,
) -> Option<()> {
    let mc_word = handle.read_config_space(msi_cap_off)?;
    let is_64bit = (mc_word & MSI_MC_64BIT_BIT) != 0;

    // Disable MSI while reprogramming so the device can't deliver during the
    // brief window between writing address and data.
    handle.write_config_space(msi_cap_off, mc_word & !MSI_MC_ENABLE_BIT)?;

    let (msg_addr, msg_data) = Arch::allocate_msi_vector(irq_vec, handler);

    handle.write_config_space(msi_cap_off + 4, msg_addr)?;
    if is_64bit {
        handle.write_config_space(msi_cap_off + 8, 0)?;
        handle.write_config_space(msi_cap_off + 0xC, msg_data)?;
    } else {
        handle.write_config_space(msi_cap_off + 8, msg_data)?;
    }

    let live_mc = handle.read_config_space(msi_cap_off)?;
    handle.write_config_space(
        msi_cap_off,
        (live_mc & !MSI_MC_MME_MASK) | MSI_MC_ENABLE_BIT,
    )?;
    Some(())
}

/*
* Registers an MSI-X handler for the given PCIe function.
* handle --- Handle into the configuration space of the given device
* bar_region: the MmioRegion corresponding to the BAR where the MSI-X table is located. This should be obtained via map_bar
* cap_offset: the offset in the PCI config space where the MSI-X capability is located. This should be obtained via get_msix_table
* table_offset: the offset within the BAR where the MSI-X table is located. This should be obtained via get_msix_table
* irq_vec: the IRQ vector to use for this MSI-X handler. If None, we will choose an arbitrary free vector.
* handler: the handler to register for this MSI-X interrupt
*/
pub fn register_msix_handler(
    handle: &mut PcieFunction,
    bar_region: &MmioRegion,
    table_offset: u32,
    cap_offset: u16,
    irq_vec: Option<u8>,
    handler: Box<dyn (Fn() -> Option<()>) + Send + Sync>,
) -> Option<()> {
    let mc_word = handle.read_config_space(cap_offset)?;
    Arch::register_msix_handler(bar_region, table_offset as u16, irq_vec, handler);
    let new_mc_word = (mc_word | 0x8000_0000) & !0x4000_0000;
    handle.write_config_space(cap_offset, new_mc_word)
}

impl Pcie {
    pub fn new(mcfg: Mcfg) -> Self {
        Self { mcfg }
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
                                let matched_device = driver.am_i_this(DeviceNode::Pcie(
                                    PcieFunction::new(bus, device, function),
                                ));
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
        for entry in self.mcfg.iterate_entries() {
            if bus >= entry.start_bus_number && bus <= entry.end_bus_number {
                let address = entry.base_address
                    + ((bus as u64) << 20)
                    + ((device as u64) << 15)
                    + ((function as u64) << 12)
                    + (offset as u64);
                return Some(unsafe { core::ptr::read_volatile(address as *const u32) });
            }
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
        for entry in self.mcfg.iterate_entries() {
            if bus >= entry.start_bus_number && bus <= entry.end_bus_number {
                let address = entry.base_address
                    + ((bus as u64) << 20)
                    + ((device as u64) << 15)
                    + ((function as u64) << 12)
                    + (offset as u64);
                unsafe { core::ptr::write_volatile(address as *mut u32, value) };
                return Some(());
            }
        }
        None
    }
}

pub fn init_pcie(mcfg: Mcfg) -> Vec<DeviceType> {
    let pcie = Pcie::new(mcfg);
    PCIE.call_once(|| pcie);
    PCIE.get().unwrap().discover()
}
