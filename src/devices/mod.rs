pub mod block;
pub mod char;
pub mod discovery;
pub mod network;
#[cfg(target_arch = "x86_64")]
pub mod usb;
pub mod virtio;

pub trait Device {
    fn ioctl(&self, request: u64, arg1: u64, arg2: u64) -> u64;
}
