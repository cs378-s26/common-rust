use alloc::vec::Vec;

use crate::print::kprintln;
use fdt::Fdt;

// a discovered hardware device from the device tree
pub struct DeviceInfo {
    pub name: &'static str,
    pub compatible: Vec<&'static str>,
    pub mmio: Option<(u64, u64)>,      // physical (base address, size in bytes) from device tree
    pub mmio_virt: Option<usize>,       // kernel virtual address after mapping; None until Step 4 maps it
}

// all devices found during boot-time device tree walk
pub struct DeviceRegistry {
    pub devices: Vec<DeviceInfo>,
}

// parses the DTB and returns a registry of every node found
pub unsafe fn walk_device_tree(dtb_ptr: *const ()) -> DeviceRegistry {
    let mut registry = DeviceRegistry { devices: Vec::new() };

    let ptr = dtb_ptr as *const u8;

    let fdt = match unsafe { Fdt::from_ptr(ptr) } {
        Ok(fdt) => fdt,
        Err(e) => {
            kprintln!("device_tree: failed to parse DTB: {:?}", e);
            return registry;
        }
    };

    kprintln!("device_tree: parsed OK, {} bytes total", fdt.total_size());

    for node in fdt.all_nodes() {
        // collect every compatible string for this node
        let compatible: Vec<&'static str> = match node.compatible() {
            Some(c) => c.all().collect(),
            None => Vec::new(),
        };

        // take the first reg region if present; most devices have exactly one
        let mmio = node.reg()
            .and_then(|mut iter| iter.next())
            .map(|region| (region.starting_address as u64, region.size.unwrap_or(0) as u64));

        registry.devices.push(DeviceInfo {
            name: node.name,
            compatible,
            mmio,
            mmio_virt: None,
        });
    }

    registry
}
