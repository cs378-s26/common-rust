use alloc::vec::Vec;

use spin::once::Once;

use crate::devices::discovery::{DeviceNode, DeviceType, SYSTEM_DRIVERS, acpi::Mcfg};
use crate::memory::dma::MmioRegion;

pub static PCIE: Once<Pcie> = Once::new();

pub struct Pcie {
    mcfg: Mcfg,
}

pub struct PcieFunction {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    //TODO: also parse i/o space bars. We are working with reasonably modern hardware, so we
    // probably won't need this
    pub bars: [Option<MmioRegion>; 6]
}

pub const PCI_CAP_MSIX: u8 = 0x11;
pub const MSI_ADDR_LAPIC0: u32 = 0xFEE0_0000;
pub const XHCI_MSI_VECTOR: u8 = 0x30;
pub const MSI_DATA_VALUE: u16 = XHCI_MSI_VECTOR as u16;

impl PcieFunction {

    fn new(bus: u8, device: u8, function: u8) -> Self {
        let mut func = Self {
            bus,
            device,
            function,
            bars: [None, None, None, None, None, None],
        };
        func.initialize_bars();
        func
    }

    pub fn read_config_space(&self, offset: u16) -> Option<u32> {
        PCIE.get().unwrap().read_config_space(self.bus, self.device, self.function, offset)
    }

    pub fn write_config_space(&self, offset: u16, value: u32) -> Option<()> {
        PCIE.get().unwrap().write_config_space(self.bus, self.device, self.function, offset, value)
    }

    fn initialize_bars(&mut self) {
        let mut bar_num = 0;
        loop {
            if bar_num >= 6 {
                break;
            }
            let cur_bar = self.read_config_space(0x10 + bar_num * 4).unwrap();
            let is_mmio = cur_bar & 1 == 0;
            if is_mmio {
                let type_ = (cur_bar >> 1) & 0b11;
                if type_ == 2 {
                    //64-bit address, so we take from the next BAR too
                    let next_bar = self.read_config_space(0x10 + (bar_num+1) * 4).unwrap();
                    let bar_addr = (cur_bar & 0xFFFFFFF0) as u64 | (((next_bar & 0xFFFFFFFF) as u64) << 32);
                    //get size
                    self.write_config_space(0x10 + bar_num * 4, 0xFFFF_FFFF);
                    self.write_config_space(0x10 + (bar_num + 1) * 4, 0xFFFF_FFFF);
                    let sz0 = (self.read_config_space(0x10 + bar_num * 4).unwrap() & 0xFFFFFFF0) as u64;
                    let sz1 = (self.read_config_space(0x10 + (bar_num + 1) * 4).unwrap() & 0xFFFFFFFF) as u64;
                    let bar_size = (!(sz0 | (sz1 << 32)) + 1) as usize;
                    //restore BARs;
                    self.write_config_space(0x10 + bar_num * 4, cur_bar);
                    self.write_config_space(0x10 + (bar_num + 1) * 4, next_bar);
                    let bar_size = bar_size.max(0x10000);
                    let bar_region = MmioRegion::new(bar_addr as usize, bar_size);
                    self.bars[bar_num as usize] = Some(bar_region);
                    self.bars[(bar_num + 1) as usize] = None; //mark the next BAR as used by this one
                    bar_num += 2;
                } else {
                    //we assume this is a 32-bit region. 
                    let bar_addr = cur_bar & 0xFFFFFFF0;
                    self.write_config_space(0x10 + bar_num * 4, 0xFFFF_FFFF);
                    let sz = (!(self.read_config_space(0x10 + bar_num * 4).unwrap() & 0xFFFFFFF0) + 1) as u64;
                    self.write_config_space(0x10 + bar_num * 4, cur_bar);
                    let bar_size = sz.max(0x10000);
                    let bar_region = MmioRegion::new(bar_addr as usize, bar_size as usize);
                    self.bars[bar_num as usize] = Some(bar_region);
                    bar_num += 1;
                }
            } else {
                //i/o space, we ignore for now
                self.bars[bar_num as usize] = None;
                bar_num += 1;
            }
        }
    }

    // setup MSI-X interrupts
    pub fn setup_msix(&self) -> bool{

        let cap_ptr_word = match self.read_config_space(0x34) {
            Some(v) => v,
            None => return false,
        };
        let mut cap_off = (cap_ptr_word & 0xFF) as u16;

        while cap_off != 0 {
            let cap_hdr = match self.read_config_space(cap_off) {
                Some(v) => v,
                None => break,
            };
            let cap_id = (cap_hdr & 0xFF) as u8;
            let next = ((cap_hdr >> 8) & 0xFF) as u16;

            if cap_id == PCI_CAP_MSIX && self.enable_msix(cap_off) {
                return true;
            }

            cap_off = next;
        }

        false
    }

    /// Configure MSI-X: write the first table entry and enable. Returns true on success.
    fn enable_msix(
        &mut self,
        cap_off: u16
    ) -> bool {
        let mc_word = match self.read_config_space(cap_off) {
            Some(v) => v,
            None => return false,
        };
        let tbl_bir_off = match self.read_config_space(cap_off + 4) {
            Some(v) => v,
            None => return false,
        };
        let bir = (tbl_bir_off & 0x7) as usize;
        let table_off = (tbl_bir_off & !0x7) as usize;
        let bar = self.bars[bir].as_mut().unwrap();
        unsafe {
            bar.write(table_off, MSI_ADDR_LAPIC0);
            bar.write(table_off + 4, 0);
            bar.write(table_off + 8, MSI_DATA_VALUE as u32);
            bar.write(table_off + 12, 0);
        };

        let new_mc_word = (mc_word | 0x8000_0000) & !0x4000_0000;
        self.write_config_space(cap_off, new_mc_word);
        true
    }

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
