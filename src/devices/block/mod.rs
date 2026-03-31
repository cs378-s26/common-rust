pub mod virtio_blk;

use crate::devices::Device;
use alloc::string::String;
use alloc::vec;

#[derive(Debug)]
pub enum BlockDeviceError {
    ReadError,
    WriteError,
    InvalidBlockIndex,
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
    fn read_block(&mut self, block_idx: usize, buffer: &mut [u8]) -> Result<(), BlockDeviceError>;
    fn write_block(&mut self, block_idx: usize, buffer: &[u8]) -> Result<(), BlockDeviceError>;

    // this allows for possible efficient buffering of reads/writes by the disk driver
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

    fn read(&mut self, byte_offset: usize, buffer: &mut [u8]) -> Result<usize, BlockDeviceError> {
        let block_size = self.block_size();
        if block_size == 0 {
            return Err(BlockDeviceError::Other("block size cannot be zero".into()));
        }
        if buffer.is_empty() {
            return Ok(0);
        }

        let mut bytes_read = 0usize;
        let mut current_offset = byte_offset;
        let mut temp = vec![0u8; block_size];

        while bytes_read < buffer.len() {
            let idx = current_offset / block_size;
            let in_block_offset = current_offset % block_size;
            let remaining = buffer.len() - bytes_read;
            let chunk_len = core::cmp::min(block_size - in_block_offset, remaining);
            let chunk = &mut buffer[bytes_read..bytes_read + chunk_len];

            let result = if in_block_offset == 0 && chunk_len == block_size {
                self.read_block(idx, chunk)
            } else {
                match self.read_block(idx, &mut temp) {
                    Ok(()) => {
                        let end = in_block_offset + chunk_len;
                        chunk.copy_from_slice(&temp[in_block_offset..end]);
                        Ok(())
                    }
                    Err(err) => Err(err),
                }
            };

            match result {
                Ok(()) => {
                    bytes_read += chunk_len;
                    current_offset = current_offset
                        .checked_add(chunk_len)
                        .ok_or(BlockDeviceError::InvalidBlockIndex)?;
                }
                Err(BlockDeviceError::ReadError) | Err(BlockDeviceError::WriteError) => break,
                Err(err) => return Err(err),
            }
        }

        Ok(bytes_read)
    }

    fn write(&mut self, byte_offset: usize, buffer: &[u8]) -> Result<usize, BlockDeviceError> {
        let block_size = self.block_size();
        if block_size == 0 {
            return Err(BlockDeviceError::Other("block size cannot be zero".into()));
        }
        if buffer.is_empty() {
            return Ok(0);
        }

        let end_offset = byte_offset
            .checked_add(buffer.len())
            .ok_or(BlockDeviceError::InvalidBlockIndex)?;
        let start_block = byte_offset / block_size;
        let start_in_block = byte_offset % block_size;
        let end_block = (end_offset - 1) / block_size;
        let end_in_block = end_offset % block_size;

        if start_block == end_block {
            if start_in_block == 0 && buffer.len() == block_size {
                return match self.write_block(start_block, buffer) {
                    Ok(()) => Ok(buffer.len()),
                    Err(BlockDeviceError::ReadError) | Err(BlockDeviceError::WriteError) => Ok(0),
                    Err(err) => Err(err),
                };
            }

            let mut temp = vec![0u8; block_size];
            match self.read_block(start_block, &mut temp) {
                Ok(()) => {}
                Err(BlockDeviceError::ReadError) | Err(BlockDeviceError::WriteError) => {
                    return Ok(0);
                }
                Err(err) => return Err(err),
            }

            let end = start_in_block + buffer.len();
            temp[start_in_block..end].copy_from_slice(buffer);
            return match self.write_block(start_block, &temp) {
                Ok(()) => Ok(buffer.len()),
                Err(BlockDeviceError::ReadError) | Err(BlockDeviceError::WriteError) => Ok(0),
                Err(err) => Err(err),
            };
        }

        let mut bytes_written = 0usize;
        let mut src_offset = 0usize;
        let mut current_block = start_block;
        let mut temp = vec![0u8; block_size];

        // First partial block.
        if start_in_block != 0 {
            let first_len = block_size - start_in_block;
            match self.read_block(current_block, &mut temp) {
                Ok(()) => {}
                Err(BlockDeviceError::ReadError) | Err(BlockDeviceError::WriteError) => {
                    return Ok(bytes_written);
                }
                Err(err) => return Err(err),
            }
            temp[start_in_block..].copy_from_slice(&buffer[..first_len]);
            match self.write_block(current_block, &temp) {
                Ok(()) => {}
                Err(BlockDeviceError::ReadError) | Err(BlockDeviceError::WriteError) => {
                    return Ok(bytes_written);
                }
                Err(err) => return Err(err),
            }
            bytes_written += first_len;
            src_offset += first_len;
            current_block = current_block
                .checked_add(1)
                .ok_or(BlockDeviceError::InvalidBlockIndex)?;
        }

        // Middle full blocks.
        let tail_exists = end_in_block != 0;
        let full_blocks_end = if tail_exists {
            end_block
        } else {
            end_block
                .checked_add(1)
                .ok_or(BlockDeviceError::InvalidBlockIndex)?
        };
        while current_block < full_blocks_end {
            let next_src_offset = src_offset
                .checked_add(block_size)
                .ok_or(BlockDeviceError::InvalidBlockIndex)?;
            let chunk = &buffer[src_offset..next_src_offset];
            match self.write_block(current_block, chunk) {
                Ok(()) => {}
                Err(BlockDeviceError::ReadError) | Err(BlockDeviceError::WriteError) => {
                    return Ok(bytes_written);
                }
                Err(err) => return Err(err),
            }
            bytes_written += block_size;
            src_offset = next_src_offset;
            current_block = current_block
                .checked_add(1)
                .ok_or(BlockDeviceError::InvalidBlockIndex)?;
        }

        // Final partial block.
        if tail_exists {
            match self.read_block(end_block, &mut temp) {
                Ok(()) => {}
                Err(BlockDeviceError::ReadError) | Err(BlockDeviceError::WriteError) => {
                    return Ok(bytes_written);
                }
                Err(err) => return Err(err),
            }
            let next_src_offset = src_offset
                .checked_add(end_in_block)
                .ok_or(BlockDeviceError::InvalidBlockIndex)?;
            temp[..end_in_block].copy_from_slice(&buffer[src_offset..next_src_offset]);
            match self.write_block(end_block, &temp) {
                Ok(()) => {}
                Err(BlockDeviceError::ReadError) | Err(BlockDeviceError::WriteError) => {
                    return Ok(bytes_written);
                }
                Err(err) => return Err(err),
            }
            bytes_written += end_in_block;
        }

        Ok(bytes_written)
    }
}
