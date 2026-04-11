pub mod discovery;
pub mod virtio_blk;
use core::ptr::NonNull;

use virtio_drivers::{BufferDirection, Hal, PhysAddr, transport::pci::bus::ConfigurationAccess};

use crate::{
    arch::{Arch, ArchTrait},
    devices::discovery::pcie::PCIE,
    memory::{
        dma::MmioRegion,
        physical_memory::{HHDM_REQUEST, alloc_frames, frame_dealloc},
    },
};

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

        let region = MmioRegion::new(phys_base, pages_covered * Arch::PAGE_SIZE);
        let virt_addr = region.virt_addr() + page_offset;

        core::mem::forget(region); // Nowhere to really keep ownership of it, we just want the mapping to stay as long as needed by driver

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

pub struct KernelConfigurationAccess;

impl ConfigurationAccess for KernelConfigurationAccess {
    fn read_word(
        &self,
        device_function: virtio_drivers::transport::pci::bus::DeviceFunction,
        register_offset: u8,
    ) -> u32 {
        PCIE.get()
            .unwrap()
            .read_config_space(
                device_function.bus,
                device_function.device,
                device_function.function,
                register_offset as u16,
            )
            .expect("Invalid read on PCI configuration space")
    }

    fn write_word(
        &mut self,
        device_function: virtio_drivers::transport::pci::bus::DeviceFunction,
        register_offset: u8,
        data: u32,
    ) {
        PCIE.get()
            .unwrap()
            .write_config_space(
                device_function.bus,
                device_function.device,
                device_function.function,
                register_offset as u16,
                data,
            )
            .expect("Invalid write on PCI configuration space")
    }

    unsafe fn unsafe_clone(&self) -> Self {
        KernelConfigurationAccess {}
    }
}
