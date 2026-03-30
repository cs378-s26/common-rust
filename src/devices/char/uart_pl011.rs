// currently the DeviceNode enum only has one variant so rust warns about using it as an if let since it's always one type,
// removing this warning for now
#![allow(irrefutable_let_patterns)]
use super::{CharDevice, CharDeviceError};
use crate::arch::{Arch, ArchTrait};
use crate::devices::Device;
use crate::devices::device_discovery::{DeviceDiscovery, DeviceNode, DeviceType};
use crate::print::{CharSink, kprint, kprintln, set_serial_backend};
use crate::virtual_memory::{PagingOptions, VirtualMemoryAllocation};
use alloc::boxed::Box;
use alloc::string::ToString;

// TODO: this driver currently is just for outputting characters to uart, later it will need to be expanded most likely
pub struct UartPl011Driver {
    phys_address: usize,
    virt_mapping: Option<VirtualMemoryAllocation>,
}

impl UartPl011Driver {
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
}

// implement char sink for the uart driver so it can be used as the serial backend
impl CharSink for UartPl011Driver {
    unsafe fn putc(&self, c: u8) {
        self.write(&[c]).expect("Failed to write character to UART");
    }

    unsafe fn flush(&self) {}
}

impl CharDevice for UartPl011Driver {
    fn read(&mut self, _buffer: &mut [u8]) -> Result<usize, CharDeviceError> {
        // TODO implement this, for now we just support output
        Err(CharDeviceError::Other(
            "Read not implemented for UART driver".to_string(),
        ))
    }

    fn write(&self, buffer: &[u8]) -> Result<usize, CharDeviceError> {
        for &b in buffer {
            if let Some(mapping) = &self.virt_mapping {
                unsafe {
                    let base = mapping.base;
                    core::ptr::write_volatile(base as *mut u8, b);
                }
            }
        }
        Ok(buffer.len())
    }
}

impl Device for UartPl011Driver {
    // for now we just return 0 for ioctl since we don't have any specific commands implemented,
    // but this can be expanded later as needed
    #[allow(unused_variables)]
    fn ioctl(&self, request: u64, arg1: u64, arg2: u64) -> u64 {
        0
    }
}

pub struct UartPl011Discovery;

impl DeviceDiscovery for UartPl011Discovery {
    // this gives full ownership of the driver to the serial backend
    fn am_i_this(&self, node: DeviceNode<'_, '_>) -> Option<DeviceType> {
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
                // TODO: need to get rid of this after x86 device discovery is up
                // initialize the driver, allocating it's virtual memory mapping
                set_serial_backend(Box::new(uart_driver));
            } else {
                panic!("Failed to initialize UART driver");
            }

            return Some(DeviceType::Special);
        }
        return None;
    }

    fn name(&self) -> &'static str {
        "Uart Pl011"
    }
}
