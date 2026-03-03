extern crate virtio_drivers;
use core::ptr::NonNull;
use virtio_drivers::transport::{Transport. mmio::MmioTransport};
use virtio_drivers::{Hal, device::blk};
use spin::Once; 

struct VirtioBlk<H: Hal, T: Transport> {
    blk: blk::VirtIOBlk<H, T>,
}

pub fn init_virtio_blk(base_addr: usize, size: usize) {
    let transport = MmioTransport::new(NonNull::new(base_addr as *mut u8).unwrap(), size);
    let blk_device = virtio_drivers::device::blk::VirtIOBlk::new(transport).unwrap();

    
}