use crate::arch::{Arch, ArchTrait};
use crate::devices::block::virtio_blk::VirtIOBlkDiskDriver;
use crate::devices::device_discovery;
use crate::devices::device_discovery::{DeviceDiscovery, DeviceNode, DeviceType};
use crate::virtual_memory::{PagingOptions, VirtualMemoryAllocation};
use alloc::boxed::Box;
use core::ptr::NonNull;
use virtio_drivers::transport::Transport;
use virtio_drivers::transport::mmio::{MmioTransport, VirtIOHeader};

pub struct VirtioDiscovery;

impl DeviceDiscovery for VirtioDiscovery {
    fn name(&self) -> &'static str {
        "virtio_discovery"
    }

    fn am_i_this(&self, node: DeviceNode) -> Option<DeviceType> {
        if let DeviceNode::DTB(fdt_node) = node
            && let Some(c) = fdt_node.compatible()
            && c.all().any(|s| s.contains("virtio,mmio"))
            && let Some(reg) = fdt_node.reg().and_then(|mut r| r.next())
        {
            let base_addr = reg.starting_address; // physical address of the MMIO region
            let size = reg.size.unwrap(); // virtio mmio device tree node should always give size of mmio header region, 512 bytes

            let hhdm_offset = crate::physical_memory::HHDM_OFFSET.get().unwrap();
            let virt_addr = base_addr as u64 + *hhdm_offset as u64;
            let virt_base = virt_addr & !(Arch::PAGE_SIZE as u64 - 1); // round down to nearest page boundary
            let phys_base = base_addr as u64 & !(Arch::PAGE_SIZE as u64 - 1); // round down to nearest page boundary

            // create temporary mapping for the MMIO region to read the device, this can later be overwritten
            // by the vm allocator, mapping made permanent below for any finalized match
            Arch::virtual_map(
                Arch::get_address_space(),
                virt_base,
                phys_base,
                PagingOptions::PRESENT | PagingOptions::WRITABLE | PagingOptions::DEVICE_MEMORY,
            );

            let header = virt_addr as *mut VirtIOHeader;
            // Safety: we just mapped this region and we trust device tree to give a valid MMIO region for a virtio device
            unsafe {
                if let Ok(transport) = MmioTransport::new(NonNull::new(header).unwrap(), size)
                    && transport.device_type() == virtio_drivers::transport::DeviceType::Block
                {
                    let options = PagingOptions::PRESENT
                        | PagingOptions::WRITABLE
                        | PagingOptions::DEVICE_MEMORY
                        | PagingOptions::SHADOW;
                    // use shadow to tell the virtual memory allocator to not reuse this mapping for anything else
                    VirtualMemoryAllocation::new(
                        Arch::get_address_space(),
                        Some(virt_base as usize),
                        size.div_ceil(Arch::PAGE_SIZE) * Arch::PAGE_SIZE, // round up to nearest page size
                        Some(phys_base as usize),
                        options,
                        false,
                    );
                    let driver = VirtIOBlkDiskDriver::new(transport);
                    return Some(device_discovery::DeviceType::Block(Box::new(driver)));
                }
            }
        }
        None
    }
}
