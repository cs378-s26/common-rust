extern crate alloc;

use crate::devices::Device;
use crate::devices::block::{BlockDevice, BlockDeviceError, PhysicalAddressSize};
use limine::request::ModuleRequest;

const PAGE_SIZE: usize = 4096;

#[unsafe(link_section = ".limine_requests")]
static MODULE_REQUEST: ModuleRequest = ModuleRequest::new();

pub trait Disk {
    fn read_sector(&self, sector: usize, buffer: &mut [u8]);
    fn write_sector(&mut self, sector: usize, buffer: &[u8]);
    fn sector_size(&self) -> usize;
}

pub struct Ramdisk<'a> {
    base_address: &'a mut [u8],
    sector_size: usize,
}

impl<'a> Ramdisk<'a> {
    pub fn new(sector_size: usize) -> Self {
        let response = MODULE_REQUEST
            .get_response()
            .expect("could not load modules (needed for fs)");
        let module = response
            .modules()
            .iter()
            .find(|module| module.string().to_bytes() == b"fs_img")
            .expect("could not find fs_img module");
        let addr = module.addr();
        let size = module.size() as usize;
        assert!(addr.align_offset(sector_size) == 0);
        assert!(size.is_multiple_of(sector_size));
        assert!(PAGE_SIZE.is_multiple_of(sector_size));
        unsafe {
            Self {
                base_address: core::slice::from_raw_parts_mut(addr, size),
                sector_size,
            }
        }
    }
}

impl<'a> Disk for Ramdisk<'a> {
    fn read_sector(&self, sector: usize, buffer: &mut [u8]) {
        assert!(sector < self.base_address.len() / self.sector_size);
        let start = sector * self.sector_size;
        let end = start + self.sector_size;
        buffer[..self.sector_size].copy_from_slice(&self.base_address[start..end]);
    }

    fn sector_size(&self) -> usize {
        self.sector_size
    }

    fn write_sector(&mut self, sector: usize, buffer: &[u8]) {
        assert!(sector < self.base_address.len() / self.sector_size);
        let start = sector * self.sector_size;
        let end = start + self.sector_size;
        self.base_address[start..end].copy_from_slice(&buffer[..self.sector_size]);
    }
}

impl Device for Ramdisk<'_> {
    fn ioctl(&self, _request: u64, _arg1: u64, _arg2: u64) -> u64 {
        0
    }
}

impl BlockDevice for Ramdisk<'_> {
    fn name(&self) -> &str {
        "ramdisk"
    }

    fn block_size(&self) -> usize {
        self.sector_size
    }

    fn block_count(&self) -> usize {
        self.base_address.len() / self.sector_size
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

        for (block_idx, buffer) in block_idxs.iter().zip(buffers.iter_mut()) {
            if *block_idx >= self.block_count() {
                return Err(BlockDeviceError::InvalidBlockIndex);
            }
            if buffer.len() != self.block_size() {
                return Err(BlockDeviceError::Other(
                    "buffer size must match block size".into(),
                ));
            }
            self.read_sector(*block_idx, buffer);
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

        for (block_idx, buffer) in block_idxs.iter().zip(buffers.iter()) {
            if *block_idx >= self.block_count() {
                return Err(BlockDeviceError::InvalidBlockIndex);
            }
            if buffer.len() != self.block_size() {
                return Err(BlockDeviceError::Other(
                    "buffer size must match block size".into(),
                ));
            }
            self.write_sector(*block_idx, buffer);
        }

        Ok(())
    }

    fn flush(&mut self) -> Result<(), BlockDeviceError> {
        Ok(())
    }

    fn dma_physical_address_size(&self) -> PhysicalAddressSize {
        PhysicalAddressSize::Size64
    }
}
