use alloc::string::String;

use crate::devices::Device;
#[cfg(target_arch = "x86_64")]
pub mod ps2_kb_m;
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
