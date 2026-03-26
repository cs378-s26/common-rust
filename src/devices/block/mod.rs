use crate::sync::IntMutex;
use alloc::boxed::Box;
use alloc::vec::Vec;
pub mod virtio_blk;

pub static FOUND_BLOCK_DEVICES: IntMutex<Vec<Box<dyn BlockDevice + Send + Sync>>> =
    IntMutex::new(Vec::new());

pub enum BlockError {
    InvalidBlockIndex,
    InvalidBufferSize,
    DeviceError,
    // add more as needed
}

pub enum PhysicalAddressSize {
    Size16,
    Size32,
    Size64,
}

pub trait BlockDevice {
    fn read_blocks(
        &mut self,
        block_idxs: &[usize],
        buffer: &mut [&mut [u8]],
    ) -> Result<(), BlockError>;
    fn write_blocks(&mut self, block_idxs: &[usize], buffer: &[&[u8]]) -> Result<(), BlockError>;
    fn flush(&mut self) -> Result<(), BlockError>;
    fn block_size(&self) -> usize;
    fn block_count(&self) -> usize;
    fn dma_physical_address_size(&self) -> PhysicalAddressSize;
}
