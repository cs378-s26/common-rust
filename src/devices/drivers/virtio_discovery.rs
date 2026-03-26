// TODO once acpi handling gets added this will need to be removed, used for device node enum that only has one enum rn
#![allow(irrefutable_let_patterns)]

use crate::arch::{Arch, ArchTrait};
use crate::devices::block::FOUND_BLOCK_DEVICES;
use crate::devices::block::virtio_blk::VirtIOBlkDiskDriver;
use crate::devices::device_discovery::{DeviceDiscovery, DeviceNode};
use crate::print::kprintln;
use crate::sync::MutexLike;
use crate::virtual_memory::{PagingOptions, VirtualMemoryAllocation};
use alloc::boxed::Box;
use core::ptr::NonNull;
use virtio_drivers::transport::mmio::{MmioTransport, VirtIOHeader};
use virtio_drivers::transport::{DeviceType, Transport};

pub struct VirtioDiscovery;

impl DeviceDiscovery for VirtioDiscovery {
    fn am_i_this(
        &self,
        node: DeviceNode,
    ) -> Option<alloc::boxed::Box<dyn crate::devices::device_discovery::DeviceDriver + Send + Sync>>
    {
        if let DeviceNode::DTB(fdt_node) = node
            && fdt_node.name.contains("virtio")
            // && let Some(c) = fdt_node.compatible()
            // && c.all().any(|s| s.contains("virtio"))
            && let Some(reg) = fdt_node.reg().and_then(|mut r| r.next())
        {

            let base_addr = reg.starting_address; // physical address of the MMIO region
            let size = reg.size.unwrap(); // virtio mmio device tree node should always give size of mmio region

            // if let Some(vm) = VirtualMemoryAllocation::new(
            //     Arch::get_address_space(),
            //     None,
            //     size.div_ceil(Arch::PAGE_SIZE) * Arch::PAGE_SIZE, // round up to nearest page size
            //     Some(base_addr as usize),
            //     PagingOptions::PRESENT | PagingOptions::WRITABLE | PagingOptions::DEVICE_MEMORY,
            // ) {
            //     let virt_addr = vm.base;
            let hhdm_offset = crate::physical_memory::HHDM_OFFSET.get().unwrap();
            let virt_addr = base_addr as u64 + *hhdm_offset as u64;
            crate::arch::Arch::virtual_map(
                Arch::get_address_space(),
                virt_addr,
                base_addr as u64,
                PagingOptions::PRESENT | PagingOptions::WRITABLE | PagingOptions::DEVICE_MEMORY,
            );

                let id = unsafe { core::ptr::read_volatile((virt_addr + 0x8) as *const u32) };
                kprintln!("id: {}", id);
                let header = virt_addr as *mut VirtIOHeader;
                // Safety: we just mapped this region and we trust device tree to give a valid MMIO region for a virtio device
                unsafe {
                    let transport = MmioTransport::new(NonNull::new(header).unwrap(), size);
                    if let Ok(transport) = transport {
                        if transport.device_type() == DeviceType::Block {
                            kprintln!("Found virtio block device, creating driver");
                            let driver = VirtIOBlkDiskDriver::new(transport);
                            FOUND_BLOCK_DEVICES.lock().push(Box::new(driver));
                        }
                    }
                }
                // core::mem::forget(vm);
            // }
        }
        None
    }
}
