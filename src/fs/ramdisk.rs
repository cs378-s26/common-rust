extern crate alloc;

use core::slice;

use crate::modules;

const PAGE_SIZE: usize = 4096;

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
        let module = modules::find_by_cmdline(b"ramdisk").unwrap_or_else(|| {
            panic!(
                "could not find ramdisk module; available modules: {:?}",
                modules::loaded_module_cmdlines().collect::<alloc::vec::Vec<_>>()
            )
        });

        let (base_ptr, len) = modules::module_range(module);
        assert!(base_ptr.align_offset(sector_size) == 0);
        assert!(len.is_multiple_of(sector_size));
        assert!(PAGE_SIZE.is_multiple_of(sector_size));
        let base_address = unsafe { slice::from_raw_parts_mut(base_ptr, len) };

        Self {
            base_address,
            sector_size,
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
