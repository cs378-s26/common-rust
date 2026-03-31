extern crate virtio_drivers;

use super::{BlockDevice, BlockDeviceError, PhysicalAddressSize};
use crate::arch::{Arch, ArchTrait};
use crate::devices::Device;
use crate::physical_memory::{HHDM_REQUEST, alloc_frames, frame_dealloc};
use crate::virtual_memory::{PagingOptions, VirtualMemoryAllocation};
use core::ptr::NonNull;
use virtio_drivers::device::blk::{SECTOR_SIZE, VirtIOBlk};
use virtio_drivers::transport::Transport;
use virtio_drivers::{BufferDirection, Hal, PhysAddr};

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

pub struct VirtioBlkHal;

unsafe impl Hal for VirtioBlkHal {
    fn dma_alloc(pages: usize, _direction: BufferDirection) -> (PhysAddr, NonNull<u8>) {
        let hhdm = HHDM_REQUEST.get_response().unwrap().offset() as usize;
        let paddr = alloc_frames(pages) as u64;
        let vaddr = paddr + hhdm as u64;

        // DT virtio-blk is DMA-coherent, so the default HHDM alias is sufficient.
        // Re-mapping it with different cacheability needs TLB shootdowns first.
        unsafe {
            core::ptr::write_bytes(vaddr as *mut u8, 0, pages * Arch::PAGE_SIZE);
        }

        (paddr, NonNull::new(vaddr as *mut u8).unwrap())
    }

    unsafe fn dma_dealloc(paddr: PhysAddr, _vaddr: NonNull<u8>, pages: usize) -> i32 {
        for page in 0..pages {
            frame_dealloc(paddr as usize + page * Arch::PAGE_SIZE);
        }
        0
    }

    unsafe fn mmio_phys_to_virt(paddr: PhysAddr, size: usize) -> NonNull<u8> {
        let phys_base = (paddr as usize) & !(Arch::PAGE_SIZE - 1);
        let page_offset = (paddr as usize) % Arch::PAGE_SIZE;
        let pages_covered = (page_offset + size).div_ceil(Arch::PAGE_SIZE);

        let mapping = VirtualMemoryAllocation::new(
            Arch::get_address_space(),
            None,
            pages_covered * Arch::PAGE_SIZE,
            Some(phys_base),
            PagingOptions::PRESENT | PagingOptions::WRITABLE | PagingOptions::DEVICE_MEMORY,
            false,
        )
        .expect("failed to allocate virtio MMIO mapping");
        let virt_addr = mapping.base + page_offset;

        core::mem::forget(mapping);
        NonNull::new(virt_addr as *mut u8).unwrap()
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

    unsafe fn unshare(paddr: PhysAddr, buffer: NonNull<[u8]>, direction: BufferDirection) {
        let hhdm = HHDM_REQUEST.get_response().unwrap().offset();
        let pages = buffer.len().div_ceil(Arch::PAGE_SIZE);
        let vaddr = NonNull::new((paddr + hhdm) as *mut u8).unwrap();

        unsafe {
            if matches!(
                direction,
                BufferDirection::DeviceToDriver | BufferDirection::Both
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
