use crate::arch::{Arch, ArchTrait};
use crate::devices::device_discovery::{DeviceDiscovery, DeviceDriver, DeviceNode, DeviceType};
use crate::print::{CharSink, kprintln};
use crate::virtual_memory::{PagingOptions, VirtualMemoryAllocation};
use alloc::boxed::Box;
use fdt::node::FdtNode;

// TODO currently this driver literally just writes out a character at a time, this will need to be expanded to include buffering and interrupts
pub struct UartPl011Driver {
    phys_address: usize,
    virt_mapping: Option<VirtualMemoryAllocation>,
}

impl DeviceDriver for UartPl011Driver {
    // defined by the driver, like uart_pl011 or virtio_blk
    fn name(&self) -> &str {
        return "uart_pl011";
    }

    fn init(&mut self) -> bool {
        let options = PagingOptions::PRESENT | PagingOptions::WRITABLE;
        let backing = Some(self.phys_address);
        let vm = VirtualMemoryAllocation::new(
            Arch::get_address_space(),
            None,
            Arch::PAGE_SIZE,
            backing,
            options,
        );
        if let Some(mapping) = vm {
            self.virt_mapping = Some(mapping);
        } else {
            return false;
        }
        return true;
    }

    fn device_type(&self) -> DeviceType {
        return DeviceType::CHAR;
    }
}

impl CharSink for UartPl011Driver {
    unsafe fn putc(&self, c: u8) {
        if let Some(mapping) = &self.virt_mapping {
            unsafe {
                let base = mapping.base;
                core::ptr::write_volatile(base as *mut u8, c);
            }
        }
    }

    unsafe fn flush(&self) {}
}

pub struct UartPl011Discovery;

impl DeviceDiscovery for UartPl011Discovery {
    fn am_i_this(&self, node: DeviceNode<'_, '_>) -> Option<Box<dyn DeviceDriver + Send + Sync>> {
        if let DeviceNode::DTB(node) = node {
            if let Some(c) = node.compatible() {
                if c.all().any(|s| s == "arm,pl011") {
                    if let Some(reg) = node.reg().and_then(|mut r| r.next()) {
                        let phys_address = reg.starting_address as usize;
                        return Some(Box::new(UartPl011Driver {
                            phys_address,
                            virt_mapping: None,
                        }));
                    }
                }
            }
        }
        None
    }
}
