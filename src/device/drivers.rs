use super::pci::{PcieConfigSpace, pcie_map_bar};
use crate::print::kprintln;
use crate::virtual_memory::{VirtualMemoryAllocation, PagingOptions};

pub fn init_e1000e(cfg : &mut PcieConfigSpace) {
    let bar0 = pcie_map_bar( cfg, 0);
    let is_eeprom = unsafe {
    core::ptr::read_volatile((bar0.base + 0x14) as *const u16)
    };
    kprintln!("eeprom: {}", is_eeprom);
}