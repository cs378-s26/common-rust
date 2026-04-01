use crate::devices::Device;
use alloc::string::String;
#[cfg(target_arch = "aarch64")]
pub mod uart_pl011;

#[derive(Debug)]
pub enum CharDeviceError {
    ReadError,
    WriteError,
    Other(String),
}

pub trait CharDevice: Device {
    fn read(&mut self, buffer: &mut [u8]) -> Result<usize, CharDeviceError>;

    fn write(&self, buffer: &[u8]) -> Result<usize, CharDeviceError>;
}
