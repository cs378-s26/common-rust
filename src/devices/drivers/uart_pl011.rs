// currently the DeviceNode enum only has one variant so rust warns about using it as an if let since it's always one type,
// removing this warning for now
#![allow(irrefutable_let_patterns)]
use crate::arch::{Arch, ArchTrait};
use crate::devices::device_discovery::{DeviceDiscovery, DeviceDriver, DeviceNode, DeviceType};
use crate::print::{CharSink, set_serial_backend};
use crate::virtual_memory::{PagingOptions, VirtualMemoryAllocation};
use alloc::boxed::Box;

// this driver currently is just for outputting characters to uart, later it will need to be expanded most likely
pub struct UartPl011Driver {
    phys_address: usize,
    virt_mapping: Option<VirtualMemoryAllocation>,
}

// implement char sink for the uart driver so it can be used as the serial backend
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

impl DeviceDriver for UartPl011Driver {
    fn name(&self) -> &str {
        "uart_pl011"
    }

    // allocate virtual memory for the UART MMIO region
    fn init(&mut self) -> bool {
        let options =
            PagingOptions::PRESENT | PagingOptions::WRITABLE | PagingOptions::DEVICE_MEMORY;
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
        true
    }

    fn device_type(&self) -> DeviceType {
        DeviceType::CHAR
    }
}

pub struct UartPl011Discovery;

impl DeviceDiscovery for UartPl011Discovery {
    // this gives full ownership of the driver to the serial backend
    // instead of returning like normal. This will likely by the pattern for prespecified devices, like block
    fn am_i_this(&self, node: DeviceNode<'_, '_>) -> Option<Box<dyn DeviceDriver + Send + Sync>> {
        if let DeviceNode::DTB(node) = node
            && let Some(c) = node.compatible()
            && c.all().any(|s| s == "arm,pl011")
            && let Some(reg) = node.reg().and_then(|mut r| r.next())
        {
            let phys_address = reg.starting_address as usize;
            let mut uart_driver = UartPl011Driver {
                phys_address,
                virt_mapping: None,
            };
            if uart_driver.init() {
                // initialize the driver, allocating it's virtual memory mapping
                set_serial_backend(Box::new(uart_driver));
            } else {
                panic!("Failed to initialize UART driver");
            }
        }
        None
    }
}
