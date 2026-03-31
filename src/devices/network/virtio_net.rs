extern crate virtio_drivers;
use crate::arch::{Arch, ArchTrait};
use crate::physical_memory::{HHDM_REQUEST, alloc_frames, frame_dealloc};
use crate::virtual_memory::{PagingOptions, VirtualMemoryAllocation};
use core::ptr::NonNull;
use virtio_drivers::{BufferDirection, Hal, PhysAddr};

// VirtioNetHal provides the kernel-side implementation of the virtio Hal trait for the net driver.
// The virtio-drivers crate is hardware-agnostic, it calls into this to allocate DMA memory and
// map MMIO regions. Mirrors VirtioBlkHal in src/devices/block/virtio_blk.rs.
pub struct VirtioNetHal;

// necessary struct for virtio net driver to communicate with hardware.
unsafe impl Hal for VirtioNetHal {
    fn dma_alloc(pages: usize, _direction: BufferDirection) -> (PhysAddr, NonNull<u8>) {
        let hhdm = HHDM_REQUEST.get_response().unwrap().offset() as usize;

        let paddr = alloc_frames(pages) as u64;
        let vaddr = paddr + hhdm as u64;
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

    unsafe fn mmio_phys_to_virt(paddr: virtio_drivers::PhysAddr, size: usize) -> NonNull<u8> {
        let hhdm = HHDM_REQUEST.get_response().unwrap().offset() as usize;

        let phys_base = (paddr as usize) & !(Arch::PAGE_SIZE - 1);
        let page_offset = (paddr as usize) % Arch::PAGE_SIZE;
        let pages_covered = (page_offset + size).div_ceil(Arch::PAGE_SIZE);
        let options = PagingOptions::PRESENT
            | PagingOptions::WRITABLE
            | PagingOptions::DEVICE_MEMORY
            | PagingOptions::SHADOW;

        VirtualMemoryAllocation::new(
            Arch::get_address_space(),
            None,
            pages_covered * Arch::PAGE_SIZE,
            Some(phys_base),
            options,
            false,
        );
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
