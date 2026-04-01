use crate::devices::block::virtio_blk::{VirtIOBlkDiskDriver, VirtioBlkHal};
use crate::devices::device_discovery::BLOCK_DEVICES;
use crate::print::kprintln;
use crate::sync::MutexLike;
use alloc::boxed::Box;
use virtio_drivers::transport::DeviceType;
use virtio_drivers::transport::pci::bus::{ConfigurationAccess, DeviceFunction, PciRoot};
use virtio_drivers::transport::pci::{PciTransport, virtio_device_type};

const PCI_CONFIG_ADDRESS: u16 = 0xCF8;
const PCI_CONFIG_DATA: u16 = 0xCFC;

/// Legacy x86 port-I/O PCI Configuration Access Mechanism (CAM-1).
/// Accesses configuration space via I/O ports 0xCF8 (address) and 0xCFC (data).
pub struct PortIoCam;

fn pci_address(df: DeviceFunction, register_offset: u8) -> u32 {
    0x80000000
        | ((df.bus as u32) << 16)
        | ((df.device as u32) << 11)
        | ((df.function as u32) << 8)
        | ((register_offset as u32) & 0xFC)
}

impl ConfigurationAccess for PortIoCam {
    fn read_word(&self, device_function: DeviceFunction, register_offset: u8) -> u32 {
        unsafe {
            x86::io::outl(PCI_CONFIG_ADDRESS, pci_address(device_function, register_offset));
            x86::io::inl(PCI_CONFIG_DATA)
        }
    }

    fn write_word(&mut self, device_function: DeviceFunction, register_offset: u8, data: u32) {
        unsafe {
            x86::io::outl(PCI_CONFIG_ADDRESS, pci_address(device_function, register_offset));
            x86::io::outl(PCI_CONFIG_DATA, data);
        }
    }

    unsafe fn unsafe_clone(&self) -> Self {
        PortIoCam
    }
}

/// Scan PCI bus 0 for virtio-blk devices and register them in BLOCK_DEVICES.
/// Only bus 0 is scanned because QEMU places all virtio-pci devices there.
/// PCI-to-PCI bridge traversal is left as a future improvement.
pub fn scan_pci_for_virtio() {
    kprintln!("[pci] scanning bus 0 for virtio devices");
    let mut root = PciRoot::new(PortIoCam);
    for (device_function, info) in root.enumerate_bus(0) {
        if virtio_device_type(&info) != Some(DeviceType::Block) {
            continue;
        }
        kprintln!(
            "[pci] found virtio-blk candidate at {} ({:04x}:{:04x})",
            device_function,
            info.vendor_id,
            info.device_id
        );
        match PciTransport::new::<VirtioBlkHal, _>(&mut root, device_function) {
            Ok(transport) => {
                kprintln!("[pci] virtio-blk initialized at {}", device_function);
                let driver = VirtIOBlkDiskDriver::new(transport);
                BLOCK_DEVICES.lock().push(Box::new(driver));
            }
            Err(e) => {
                kprintln!(
                    "[pci] virtio-blk at {} failed to initialize: {:?}",
                    device_function,
                    e
                );
            }
        }
    }
}
