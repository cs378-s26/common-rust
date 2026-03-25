pub mod virtio_blk;

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
    // buffers instead of vecs to avoid needing memory allocation, but maybe this isn't necessary
    fn init() -> Result<(), BlockError>;
    fn read_blocks(&self, block_idxs: &[usize], buffer: &[&mut [u8]]) -> Result<(), BlockError>;
    fn write_blocks(&self, block_idxs: &[usize], buffer: &[&[u8]]) -> Result<(), BlockError>;
    fn flush(&self) -> Result<(), BlockError>;
    fn block_size(&self) -> usize;
    fn block_count(&self) -> usize;
    fn dma_physical_address_size(&self) -> PhysicalAddressSize;
}