use crate::devices::device_discovery::{AcpiDeviceNode, DeviceDiscovery, DeviceNode, DeviceType};
use crate::devices::{Device, char::CharDevice, char::CharDeviceError};
use crate::print::{CharSink, set_serial_backend};
use alloc::boxed::Box;
use core::cell::SyncUnsafeCell;
use uart_16550::SerialPort;

pub struct Uart16550Driver {
    port: SyncUnsafeCell<SerialPort>,
}

impl Uart16550Driver {
    fn new(port: u16) -> Self {
        let mut serial = unsafe { SerialPort::new(port) };
        serial.init();
        Self { port: SyncUnsafeCell::new(serial) }
    }
}

impl CharSink for Uart16550Driver {
    unsafe fn putc(&self, ch: u8) {
        unsafe { &mut *self.port.get() }.send(ch);
    }

    unsafe fn flush(&self) {}
}

impl CharDevice for Uart16550Driver {
    fn read(&mut self, _buffer: &mut [u8]) -> Result<usize, CharDeviceError> {
        Err(CharDeviceError::Other(alloc::string::String::from("read not implemented")))
    }

    fn write(&self, buffer: &[u8]) -> Result<usize, CharDeviceError> {
        for &b in buffer {
            unsafe { &mut *self.port.get() }.send(b);
        }
        Ok(buffer.len())
    }
}

impl Device for Uart16550Driver {
    fn ioctl(&self, _request: u64, _arg1: u64, _arg2: u64) -> u64 {
        0
    }
}

pub struct Uart16550Discovery;

impl DeviceDiscovery for Uart16550Discovery {
    fn am_i_this(&self, node: DeviceNode) -> Option<DeviceType> {
        if let DeviceNode::Acpi(AcpiDeviceNode::Serial16550 { port }) = node {
            set_serial_backend(Box::new(Uart16550Driver::new(port)));
            Some(DeviceType::Special)
        } else {
            None
        }
    }
}
