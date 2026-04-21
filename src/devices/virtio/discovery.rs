use alloc::{boxed::Box, vec, vec::Vec};
use core::ptr::NonNull;

use virtio_drivers::transport::{
    Transport,
    mmio::{MmioTransport, VirtIOHeader},
    pci::{
        PciTransport,
        bus::{DeviceFunction, PciRoot},
    },
};
use crate::print::kprintln;

use crate::devices::{
    block::virtio_blk::VirtIOBlkDiskDriver,
    discovery::{self, DeviceDiscovery, DeviceNode, DeviceType},
    network::virtio_net::VirtIONetDriver,
    virtio::{KernelConfigurationAccess, VirtioHal},
};

pub struct VirtioDiscovery;

impl DeviceDiscovery for VirtioDiscovery {
    fn am_i_this(&self, node: DeviceNode) -> Option<Vec<DeviceType>> {
        if let DeviceNode::DTB(fdt_node) = node
            && let Some(c) = fdt_node.compatible()
            && c.all().any(|s| s.contains("virtio,mmio"))
            && let Some(reg) = fdt_node.reg().and_then(|mut r| r.next())
        {
            let base_addr = reg.starting_address; // physical address of the MMIO region
            let size = reg.size.unwrap(); // virtio mmio device tree node should always give size of mmio header region, 512 bytes

            // TODO this is making many permanent mappings of the same region, we should ideally be reusing if it is already mapped
            let header: NonNull<VirtIOHeader> = super::map_mmio(base_addr as usize, size).cast();
            // safety: we trust the device tree to give a valid mmio region for a virtio device
            unsafe {
                if let Ok(transport) = MmioTransport::new(header, size) {
                    match transport.device_type() {
                        virtio_drivers::transport::DeviceType::Block => {
                            let driver = VirtIOBlkDiskDriver::new(transport);
                            return Some(vec![discovery::DeviceType::Block(Box::new(driver))]);
                        }
                        virtio_drivers::transport::DeviceType::Network => {
                            let driver = VirtIONetDriver::<VirtioHal, _, 16>::new(transport);
                            return Some(vec![discovery::DeviceType::Network(Box::new(driver))]);
                        }
                        _ => {}
                    }
                }
            }
        } else if let DeviceNode::Pcie(pcie_fn) = node {
            let mut pci_root = PciRoot::new(KernelConfigurationAccess {});
            let transport = PciTransport::new::<VirtioHal, KernelConfigurationAccess>(
                &mut pci_root,
                DeviceFunction {
                    bus: pcie_fn.bus,
                    device: pcie_fn.device,
                    function: pcie_fn.function,
                },
            )
            .ok()?;
            kprintln!("found virtio pcie device");
            kprintln!("{:?}", transport.device_type());

            match transport.device_type() {
                virtio_drivers::transport::DeviceType::Block => {
                    let driver = VirtIOBlkDiskDriver::new(transport);
                    return Some(vec![discovery::DeviceType::Block(Box::new(driver))]);
                }
                virtio_drivers::transport::DeviceType::Network => {
                    let driver = VirtIONetDriver::<VirtioHal, _, 16>::new(transport);
                    kprintln!("weee");
                    return Some(vec![discovery::DeviceType::Network(Box::new(driver))]);
                }
                _ => {}
            }
        }
        None
    }

    fn name(&self) -> &'static str {
        "virtio_discovery"
    }
}
