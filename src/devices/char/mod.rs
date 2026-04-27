use alloc::string::String;
use crate::fs::vfs::{VFSDevice, FsError};

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


impl VFSDevice for dyn CharDevice {
    fn read_unaligned(&mut self, _ : usize, buffer: &mut [u8]) -> Result<usize, FsError> {
        self.read(buffer).map_err(|_| FsError::ReadError)
    }

    fn write_unaligned(&mut self, _ : usize, buffer: &[u8]) -> Result<usize, FsError> {
        self.write(buffer).map_err(|_| FsError::WriteError)
    }
}