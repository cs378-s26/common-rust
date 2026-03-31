extern crate virtio_drivers;
// use super::{BlockDevice, BlockError, PhysicalAddressSize};
use crate::arch::{Arch, ArchTrait};
use crate::devices::Device;
use super::{BlockDevice, BlockDeviceError, PhysicalAddressSize};
use crate::physical_memory::{HHDM_REQUEST, alloc_frames, frame_dealloc};
use crate::virtual_memory::PagingOptions;
use core::ptr::NonNull;
use virtio_drivers::device::blk::{SECTOR_SIZE, VirtIOBlk};
use virtio_drivers::transport::Transport;
use virtio_drivers::{BufferDirection, Hal, PhysAddr};
// wrapper around the virtio blk driver containing the necessary hal implementation for it to work
// with our system + the system block device trait
// view crate and specs here: https://docs.rs/virtio-drivers/latest/virtio_drivers/ 
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
}

impl<T: Transport> BlockDevice for VirtIOBlkDiskDriver<VirtioBlkHal, T> {
    fn name(&self) -> &str {
        "virtio_blk"
    }

    fn read_block(&mut self, block_idx: usize, buffer: &mut [u8]) -> Result<(), BlockDeviceError> {
        check_buffer_size(buffer, self.block_size())?;
        let sectors_per_block = self.block_size() / SECTOR_SIZE;
        let sector_idx = block_idx * sectors_per_block;
        self.blk
            .read_blocks(sector_idx, buffer) // this uses virtio driver's internal read_blocks, which is synchronous
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
            self.read_block(*block_idx, *buf)
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
            self.write_block(*block_idx, *buf)
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
        // No special ioctls implemented for the virtio block device yet.
        0
    }
}

fn check_buffer_size(buffer: &[u8], block_size: usize) -> Result<(), BlockDeviceError> {
    if buffer.len() != block_size {
        return Err(BlockDeviceError::Other(
            "buffer size must be equal to block size".into(),
        ));
    }
    Ok(())
}

pub struct VirtioBlkHal;

// necessary struct for virtio driver to communicate with hardware.
unsafe impl Hal for VirtioBlkHal {
    // buffer direction is used to determine writability and readability of the buffer between device and host, for now ignored
    fn dma_alloc(pages: usize, _direction: BufferDirection) -> (PhysAddr, NonNull<u8>) {
        let hhdm = HHDM_REQUEST.get_response().unwrap().offset() as usize;

        let options = PagingOptions::PRESENT | PagingOptions::WRITABLE;
        let paddr = alloc_frames(pages) as u64;
        let vaddr = paddr + hhdm as u64;

        // map the allocated frames to be non-cacheable, then map them back to normal on deallocation
        // this avoids cache coherency issues with the device, but in the future we may want to handle proper cache coherency
        for page in 0..pages {
            Arch::virtual_map(
                Arch::get_address_space(),
                (vaddr as usize + page * Arch::PAGE_SIZE) as u64,
                (paddr as usize + page * Arch::PAGE_SIZE) as u64,
                options,
            );
        }
        // zero the frames
        unsafe {
            core::ptr::write_bytes(vaddr as *mut u8, 0, pages * Arch::PAGE_SIZE);
        }
        // return phys adddress and non null virtual maped address
        (paddr, NonNull::new(vaddr as *mut u8).unwrap())
    }

    unsafe fn dma_dealloc(paddr: PhysAddr, vaddr: NonNull<u8>, pages: usize) -> i32 {
        let options = PagingOptions::PRESENT | PagingOptions::WRITABLE | PagingOptions::CACHEABLE;
        for page in 0..pages {
            // reset permissions, then free the frame so it can be used by other things.
            Arch::virtual_map(
                Arch::get_address_space(),
                (vaddr.as_ptr() as usize + page * Arch::PAGE_SIZE) as u64,
                (paddr as usize + page * Arch::PAGE_SIZE) as u64,
                options,
            );
            frame_dealloc(paddr as usize + page * Arch::PAGE_SIZE);
        }
        0
    }

    // maps a physical mmio region to a virtual address, must be mapped
    unsafe fn mmio_phys_to_virt(paddr: virtio_drivers::PhysAddr, size: usize) -> NonNull<u8> {
        let hhdm = HHDM_REQUEST.get_response().unwrap().offset() as usize;

        // get the total amount of pages covered by the region and
        let page_offset = (paddr as usize) % Arch::PAGE_SIZE;
        let pages_covered = (page_offset + size).div_ceil(Arch::PAGE_SIZE);

        for page in 0..pages_covered {
            Arch::virtual_map(
                Arch::get_address_space(),
                (paddr as usize + page * Arch::PAGE_SIZE + hhdm) as u64,
                (paddr as usize + page * Arch::PAGE_SIZE) as u64,
                PagingOptions::PRESENT | PagingOptions::WRITABLE | PagingOptions::DEVICE_MEMORY, 
            );
        }
        NonNull::new((paddr as usize + hhdm) as *mut u8).unwrap()
    }

    unsafe fn share(buffer: NonNull<[u8]>, direction: BufferDirection) -> PhysAddr {
        let pages = buffer.len().div_ceil(Arch::PAGE_SIZE);
        let (paddr, _) = Self::dma_alloc(pages, direction);
        let hhdm = HHDM_REQUEST.get_response().unwrap().offset();
        if matches!(
            direction,
            BufferDirection::DriverToDevice | BufferDirection::Both
        ) {
            unsafe {
                core::ptr::copy_nonoverlapping(
                    buffer.as_ptr() as *const u8,
                    (paddr + hhdm) as *mut u8,
                    buffer.len(),
                );
            }
        }
        paddr
    }

    unsafe fn unshare(
        paddr: virtio_drivers::PhysAddr,
        buffer: NonNull<[u8]>,
        direction: virtio_drivers::BufferDirection,
    ) {
        let hhdm = HHDM_REQUEST.get_response().unwrap().offset();
        let pages = buffer.len().div_ceil(Arch::PAGE_SIZE);
        let vaddr = NonNull::new((paddr + hhdm) as *mut u8).unwrap();
        unsafe {
            if matches!(
                direction,
                virtio_drivers::BufferDirection::DeviceToDriver
                    | virtio_drivers::BufferDirection::Both
            ) {
                core::ptr::copy_nonoverlapping(
                    vaddr.as_ptr() as *const u8,
                    buffer.as_ptr() as *mut u8,
                    buffer.len(),
                );
            }
            Self::dma_dealloc(paddr, vaddr, pages);
        }
    }
}
