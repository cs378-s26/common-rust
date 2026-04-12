extern crate virtio_drivers;

use crate::arch::{Arch, ArchTrait};
use crate::devices::Device;
use crate::devices::block::{BlockDevice, BlockDeviceError, PhysicalAddressSize};
use crate::devices::virtio::VirtioBlkHal;
use virtio_drivers::Hal;
use virtio_drivers::device::blk::{SECTOR_SIZE, VirtIOBlk};
use virtio_drivers::transport::Transport;

// Wrapper around the virtio blk driver containing the necessary HAL
// implementation for it to work with our system block device trait.
pub struct VirtIOBlkDiskDriver<H: Hal, T: Transport> {
    blk: VirtIOBlk<H, T>,
}

impl<T: Transport> VirtIOBlkDiskDriver<VirtioBlkHal, T> {
    pub fn new(transport: T) -> Self {
        Self {
            blk: VirtIOBlk::<VirtioBlkHal, T>::new(transport)
                .expect("failed to initialize virtio blk device"),
        }
    }

    fn read_block(&mut self, block_idx: usize, buffer: &mut [u8]) -> Result<(), BlockDeviceError> {
        check_buffer_size(buffer, self.block_size())?;
        let sectors_per_block = self.block_size() / SECTOR_SIZE;
        let sector_idx = block_idx * sectors_per_block;
        self.blk
            .read_blocks(sector_idx, buffer)
            .map_err(|_| BlockDeviceError::ReadError)?;
        Ok(())
    }

    fn write_block(&mut self, block_idx: usize, buffer: &[u8]) -> Result<(), BlockDeviceError> {
        check_buffer_size(buffer, self.block_size())?;
        let sectors_per_block = self.block_size() / SECTOR_SIZE;
        let sector_idx = block_idx * sectors_per_block;
        self.blk
            .write_blocks(sector_idx, buffer)
            .map_err(|_| BlockDeviceError::WriteError)?;
        Ok(())
    }
}

impl<T: Transport> BlockDevice for VirtIOBlkDiskDriver<VirtioBlkHal, T> {
    fn name(&self) -> &str {
        "virtio_blk"
    }

    fn read_blocks(
        &mut self,
        block_idxs: &[usize],
        buffers: &mut [&mut [u8]],
    ) -> Result<(), BlockDeviceError> {
        if block_idxs.len() != buffers.len() {
            return Err(BlockDeviceError::Other(
                "buffer count must match indexes".into(),
            ));
        }

        for (block_idx, buf) in block_idxs.iter().zip(buffers.iter_mut()) {
            self.read_block(*block_idx, buf)
                .map_err(|_| BlockDeviceError::ReadError)?;
        }

        Ok(())
    }

    fn write_blocks(
        &mut self,
        block_idxs: &[usize],
        buffers: &[&[u8]],
    ) -> Result<(), BlockDeviceError> {
        if block_idxs.len() != buffers.len() {
            return Err(BlockDeviceError::Other(
                "buffer count must match indexes".into(),
            ));
        }

        for (block_idx, buf) in block_idxs.iter().zip(buffers.iter()) {
            self.write_block(*block_idx, buf)
                .map_err(|_| BlockDeviceError::WriteError)?;
        }

        Ok(())
    }

    fn flush(&mut self) -> Result<(), BlockDeviceError> {
        self.blk
            .flush()
            .map_err(|_| BlockDeviceError::Other("flush failed".into()))?;
        Ok(())
    }

    fn block_size(&self) -> usize {
        Arch::PAGE_SIZE
    }

    fn block_count(&self) -> usize {
        (self.blk.capacity() as usize * SECTOR_SIZE) / self.block_size()
    }

    fn dma_physical_address_size(&self) -> PhysicalAddressSize {
        PhysicalAddressSize::Size64
    }
}

impl<T: Transport> Device for VirtIOBlkDiskDriver<VirtioBlkHal, T> {
    #[allow(unused_variables)]
    fn ioctl(&self, request: u64, arg1: u64, arg2: u64) -> u64 {
        0
    }
}

fn check_buffer_size(buffer: &[u8], block_size: usize) -> Result<(), BlockDeviceError> {
    if buffer.len() != block_size {
        return Err(BlockDeviceError::InvalidBufferSize);
    }
    Ok(())
}
