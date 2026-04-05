pub mod block;
pub mod char;
pub mod discovery;
pub mod network;
pub mod virtio_discovery;

pub trait Device {
    fn ioctl(&self, request: u64, arg1: u64, arg2: u64) -> u64;
}
