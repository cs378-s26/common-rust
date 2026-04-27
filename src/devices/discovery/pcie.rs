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
    pub bars: [Option<MmioRegion>; 6]
}

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
                                    driver.am_i_this(DeviceNode::Pcie(PcieFunction::new(bus, device, function)));
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
