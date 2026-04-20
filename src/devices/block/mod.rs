pub mod virtio_blk;
use alloc::{string::String, vec, vec::Vec};

use crate::devices::Device;

#[derive(Debug)]
pub enum BlockDeviceError {
    ReadError,
    WriteError,
    InvalidBlockIndex,
    InvalidBufferSize,
    Other(String),
}

pub enum PhysicalAddressSize {
    Size16,
    Size32,
    Size64,
}

pub trait BlockDevice: Device {
    fn name(&self) -> &str;
    fn block_size(&self) -> usize;
    fn block_count(&self) -> usize;

    // This keeps the lower-level interface allocation-free for callers while
    // still allowing drivers to batch whole-block transfers efficiently.
    fn read_blocks(
        &mut self,
        block_idxs: &[usize],
        buffers: &mut [&mut [u8]],
    ) -> Result<(), BlockDeviceError>;

    fn write_blocks(
        &mut self,
        block_idxs: &[usize],
        buffers: &[&[u8]],
    ) -> Result<(), BlockDeviceError>;

    fn flush(&mut self) -> Result<(), BlockDeviceError>;
    fn dma_physical_address_size(&self) -> PhysicalAddressSize;

    // read starting from some byte offset until buffer is full
    fn read(&mut self, byte_offset: usize, buffer: &mut [u8]) -> Result<usize, BlockDeviceError> {
        let block_size = self.block_size();
        if block_size == 0 {
            return Err(BlockDeviceError::Other("block size cannot be zero".into()));
        }
        if buffer.is_empty() {
            return Ok(0);
        }
        let mut total_read = 0;

        let start_idx = byte_offset / block_size;
        let end_idx = ((byte_offset + buffer.len() - 1) / block_size) + 1;

        // this is for handling partial block reads/writes
        let start_offset = byte_offset % block_size;
        let end_offset = (byte_offset + buffer.len()) % block_size;

        let first_full_idx = if start_offset == 0 {
            start_idx
        } else {
            start_idx + 1
        };

        let last_full_idx = if end_offset == 0 {
            end_idx
        } else {
            end_idx - 1
        };

        // if there are middle blocks that can be read in full, read them batched for efficiency
        if first_full_idx < last_full_idx {
            // inclusive list of block indices that can be read/written in full
            let mid_idxs: Vec<usize> = (first_full_idx..last_full_idx).collect();
            let mid_start = mid_idxs[0] * block_size;
            let mid_end = (mid_idxs[mid_idxs.len() - 1] + 1) * block_size;

            // currently mid_start and mid_end are bytes within the block device, subtract byte_offset
            // to get the corresponding place within the caller's buffer
            let mid_buffer = &mut buffer[mid_start - byte_offset..mid_end - byte_offset];
            let mut mid_buffers: Vec<&mut [u8]> = mid_buffer.chunks_exact_mut(block_size).collect();

            // vecs get changed to be slices to match function call
            self.read_blocks(&mid_idxs, &mut mid_buffers)?;
            total_read += mid_buffer.len();
        }

        // read in partial first block. If whole read is within one block, this is the code that should run
        if first_full_idx > start_idx || (first_full_idx == last_full_idx) {
            let mut temp = vec![0u8; block_size];
            self.read_blocks(&[start_idx], &mut [temp.as_mut_slice()])?;

            // copy from the offset within the block until either the end of the block or the end of the given buffer, whichever comes first
            let copy_len = core::cmp::min(block_size - start_offset, buffer.len());
            buffer[..copy_len].copy_from_slice(&temp[start_offset..start_offset + copy_len]);
            total_read += copy_len;
        }

        // read in partial last block if it's different from the first block
        if last_full_idx < end_idx && start_idx != end_idx - 1 {
            let mut temp = vec![0u8; block_size];
            self.read_blocks(&[end_idx - 1], &mut [temp.as_mut_slice()])?;
            let copy_len = end_offset;
            let buffer_len = buffer.len();
            buffer[buffer_len - copy_len..].copy_from_slice(&temp[..copy_len]);
            total_read += copy_len;
        }

        Ok(total_read)
    }

    fn write(&mut self, byte_offset: usize, buffer: &[u8]) -> Result<usize, BlockDeviceError> {
        let block_size = self.block_size();
        if block_size == 0 {
            return Err(BlockDeviceError::Other("block size cannot be zero".into()));
        }
        if buffer.is_empty() {
            return Ok(0);
        }

        let end_byte = byte_offset
            .checked_add(buffer.len())
            .ok_or(BlockDeviceError::Other("offset overflow".into()))?;

        let mut total_written = 0;

        let start_idx = byte_offset / block_size;
        let end_idx = ((end_byte - 1) / block_size) + 1; // exclusive

        let start_offset = byte_offset % block_size;
        let end_offset = end_byte % block_size;

        let first_full_idx = if start_offset == 0 {
            start_idx
        } else {
            start_idx + 1
        };

        let last_full_idx = if end_offset == 0 {
            end_idx
        } else {
            end_idx - 1
        };

        // write any fully covered middle blocks directly from the caller's buffer.
        if first_full_idx < last_full_idx {
            let mid_idxs: Vec<usize> = (first_full_idx..last_full_idx).collect();
            let mid_start = mid_idxs[0] * block_size;
            let mid_end = (mid_idxs[mid_idxs.len() - 1] + 1) * block_size;

            let mid_buffer = &buffer[mid_start - byte_offset..mid_end - byte_offset];
            let mid_buffers: Vec<&[u8]> = mid_buffer.chunks_exact(block_size).collect();

            self.write_blocks(&mid_idxs, &mid_buffers)?;
            total_written += mid_buffer.len();
        }

        // handle partial first block. If the whole write is within one block,
        // this is the branch that should run.
        if first_full_idx > start_idx || (first_full_idx == last_full_idx) {
            let mut temp = vec![0u8; block_size];
            self.read_blocks(&[start_idx], &mut [temp.as_mut_slice()])?;

            let copy_len = core::cmp::min(block_size - start_offset, buffer.len());
            temp[start_offset..start_offset + copy_len].copy_from_slice(&buffer[..copy_len]);

            self.write_blocks(&[start_idx], &[temp.as_slice()])?;
            total_written += copy_len;
        }

        // handle partial last block if it is different from the first block.
        if last_full_idx < end_idx && start_idx != end_idx - 1 {
            let mut temp = vec![0u8; block_size];
            self.read_blocks(&[end_idx - 1], &mut [temp.as_mut_slice()])?;

            let copy_len = end_offset;
            let buffer_len = buffer.len();
            temp[..copy_len].copy_from_slice(&buffer[buffer_len - copy_len..]);

            self.write_blocks(&[end_idx - 1], &[temp.as_slice()])?;
            total_written += copy_len;
        }

        Ok(total_written)
    }
}
