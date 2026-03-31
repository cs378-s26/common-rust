use crate::devices::Device;
use alloc::string::String;

#[derive(Debug)]
pub enum NetworkError {
    SendError,
    ReceiveError,
    BufferTooSmall,
    Other(String),
}

pub trait NetworkDevice: Device {
    fn name(&self) -> &str;
    fn send_packet(&mut self, packet: &[u8]) -> Result<(), NetworkError>;
    // returns the number of bytes written into buffer
    fn receive_packet(&mut self, buffer: &mut [u8]) -> Result<usize, NetworkError>;
}
