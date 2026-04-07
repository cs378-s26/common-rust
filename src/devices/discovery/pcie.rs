use crate::devices::discovery::{DeviceNode, DeviceType, SYSTEM_DRIVERS, acpi::Mcfg};
use alloc::vec::Vec;
use spin::once::Once;

pub static PCIE: Once<Pcie> = Once::new();

pub struct Pcie {
    mcfg: Mcfg,
}

pub struct PcieFunction {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
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
