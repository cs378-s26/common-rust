use crate::devices::block::virtio_blk::{VirtIOBlkDiskDriver, VirtioBlkHal};
use crate::devices::discovery::acpi::McfgEntry;
use crate::devices::discovery::{DeviceDiscovery, DeviceNode, DeviceType};
use crate::dma::MmioRegion;
use crate::print::kprintln;
use alloc::boxed::Box;
use alloc::vec::Vec;
use virtio_drivers::transport::DeviceType as VirtioDeviceType;
use virtio_drivers::transport::pci::bus::{ConfigurationAccess, DeviceFunction, PciRoot};
use virtio_drivers::transport::pci::{PciTransport, virtio_device_type};

/// ECAM (Enhanced Configuration Access Mechanism) for PCIe config space.
/// Uses the memory-mapped base address from the ACPI MCFG table.
struct EcamCam {
    base_virt: usize,
    start_bus: u8,
}

impl EcamCam {
    // Takes McfgEntry by value so packed field copies happen at the call site.
    fn new(entry: McfgEntry) -> Self {
        let base_address = entry.base_address;
        let start_bus = entry.start_bus;
        let end_bus = entry.end_bus;
        let size = (end_bus as usize - start_bus as usize + 1) << 20;
        let region = MmioRegion::new(base_address as usize, size);
        let base_virt = region.virt_addr();
        core::mem::forget(region); // mapping must outlive all config space accesses
        EcamCam { base_virt, start_bus }
    }

    fn config_ptr(&self, df: DeviceFunction, register_offset: u8) -> *mut u32 {
        let addr = self.base_virt
            + ((df.bus as usize - self.start_bus as usize) << 20)
            + ((df.device as usize) << 15)
            + ((df.function as usize) << 12)
            + (register_offset as usize & 0xFFC);
        addr as *mut u32
    }
}

impl ConfigurationAccess for EcamCam {
    fn read_word(&self, device_function: DeviceFunction, register_offset: u8) -> u32 {
        unsafe { core::ptr::read_volatile(self.config_ptr(device_function, register_offset)) }
    }

    fn write_word(&mut self, device_function: DeviceFunction, register_offset: u8, data: u32) {
        unsafe { core::ptr::write_volatile(self.config_ptr(device_function, register_offset), data) }
    }

    unsafe fn unsafe_clone(&self) -> Self {
        EcamCam {
            base_virt: self.base_virt,
            start_bus: self.start_bus,
        }
    }
}

/// Discovers virtio-blk devices on the PCI bus via ECAM.
/// Triggered by a `DeviceNode::Mcfg` node — iterates over all MCFG allocation
/// structures and scans each bus range for virtio-blk devices.
pub struct PciVirtioDiscovery;

impl DeviceDiscovery for PciVirtioDiscovery {
    fn am_i_this(&self, node: DeviceNode) -> Option<Vec<DeviceType>> {
        let DeviceNode::Mcfg(mcfg) = node else {
            return None;
        };

        let mut devices = Vec::new();

        for entry in mcfg.iterate_entries() {
            // Copy packed fields to locals before any use.
            let base_address = entry.base_address;
            let segment_group = entry.segment_group;
            let start_bus = entry.start_bus;
            let end_bus = entry.end_bus;
            kprintln!(
                "[pci] scanning segment {} buses {}..{} (ecam base {:#x})",
                segment_group,
                start_bus,
                end_bus,
                base_address
            );
            let cam = EcamCam::new(entry);
            let mut root = PciRoot::new(cam);

            for bus in start_bus..=end_bus {
                for (device_function, info) in root.enumerate_bus(bus) {
                    if virtio_device_type(&info) != Some(VirtioDeviceType::Block) {
                        continue;
                    }
                    kprintln!(
                        "[pci] found virtio-blk at {} ({:04x}:{:04x})",
                        device_function,
                        info.vendor_id,
                        info.device_id
                    );
                    match PciTransport::new::<VirtioBlkHal, _>(&mut root, device_function) {
                        Ok(transport) => {
                            kprintln!("[pci] virtio-blk initialized at {}", device_function);
                            devices.push(DeviceType::Block(Box::new(VirtIOBlkDiskDriver::new(
                                transport,
                            ))));
                        }
                        Err(e) => {
                            kprintln!("[pci] virtio-blk at {} failed: {:?}", device_function, e);
                        }
                    }
                }
            }
        }

        Some(devices)
    }

    fn name(&self) -> &'static str {
        "pci_virtio"
    }
}
