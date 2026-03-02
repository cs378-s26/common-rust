extern crate alloc;

use limine::request::ModuleRequest;

const PAGE_SIZE: usize = 4096;

#[used]
#[unsafe(link_section = ".limine_requests")]
static MODULE_REQUEST: ModuleRequest = ModuleRequest::new();

pub trait Disk {
    fn read_sector(self: &Self, sector: usize, buffer: &mut [u8]);
    fn write_sector(self: &mut Self, sector: usize, buffer: &[u8]);
    fn sector_size(self: &Self) -> usize;
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
        let module = response.modules()[1];
        let addr = module.addr();
        let size = module.size() as usize;
        assert!(addr.align_offset(sector_size) == 0);
        assert!(size % sector_size == 0);
        assert!(PAGE_SIZE % sector_size == 0);
        unsafe {
            return Self {
                base_address: core::slice::from_raw_parts_mut(addr, size),
                sector_size: sector_size,
            };
        }
    }
}

impl<'a> Disk for Ramdisk<'a> {
    fn read_sector(self: &Self, sector: usize, buffer: &mut [u8]) {
        assert!(sector < self.base_address.len() / self.sector_size);
        let start = sector * self.sector_size;
        let end = start + self.sector_size;
        buffer[..self.sector_size].copy_from_slice(&self.base_address[start..end]);
    }

    fn sector_size(self: &Self) -> usize {
        self.sector_size
    }

    fn write_sector(self: &mut Self, sector: usize, buffer: &[u8]) {
        assert!(sector < self.base_address.len() / self.sector_size);
        let start = sector * self.sector_size;
        let end = start + self.sector_size;
        self.base_address[start..end].copy_from_slice(&buffer[..self.sector_size]);
    }
}
