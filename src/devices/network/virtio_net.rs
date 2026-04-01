extern crate virtio_drivers;
use super::{NetworkDevice, NetworkError};
use crate::arch::{Arch, ArchTrait};
use crate::devices::Device;
use crate::physical_memory::{HHDM_REQUEST, alloc_frames, frame_dealloc};
use crate::virtual_memory::{PagingOptions, VirtualMemoryAllocation};
use core::ptr::NonNull;
use virtio_drivers::device::net::VirtIONet;
use virtio_drivers::transport::Transport;
use virtio_drivers::{BufferDirection, Hal, PhysAddr};

/// VirtIONetDriver wraps the virtio-drivers VirtIONet device and ties it to our kernel's HAL
/// constructed from a transport (MMIO) and used by the device framework to send and receive packets
pub struct VirtIONetDriver<H: Hal, T: Transport> {
    net: VirtIONet<H, T>,
}

impl<T: Transport> VirtIONetDriver<VirtioNetHal, T> {
    pub fn new(transport: T) -> Self {
        Self {
            net: VirtIONet::<VirtioNetHal, T>::new(transport)
                .expect("failed to initialize virtio net device"),
        }
    }
}

/// VirtioNetHal provides the kernel-side implementation of the virtio Hal trait for the net driver
/// the virtio-drivers crate is hardware-agnostic, it calls into this to allocate DMA memory and
/// map MMIO regions. Mirrors VirtioBlkHal in src/devices/block/virtio_blk.rs
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
        0 // 0 = success, required by Hal trait (C-style status code)
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

        // mark the physical region as in-use in the VM allocator so it won't be
        // handed out for other allocations. the actual pointer we return is the
        // HHDM direct map address, not the newly allocated VA
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

    // allocates a DMA bounce buffer and copies data into it if the direction is
    // driver-to-device. returns the physical address for the device to read from.
    // the bounce buffer is freed by unshare when the transfer is complete
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

impl<T: Transport> NetworkDevice for VirtIONetDriver<VirtioNetHal, T> {
    fn name(&self) -> &str {
        "virtio_net"
    }

    fn send_packet(&mut self, packet: &[u8]) -> Result<(), NetworkError> {
        self.net.send(packet).map_err(|_| NetworkError::SendError)
    }

    fn receive_packet(&mut self, buffer: &mut [u8]) -> Result<usize, NetworkError> {
        self.net.receive(buffer).map_err(|_| NetworkError::ReceiveError)
    }
}

impl<T: Transport> Device for VirtIONetDriver<VirtioNetHal, T> {
    #[allow(unused_variables)]
    fn ioctl(&self, request: u64, arg1: u64, arg2: u64) -> u64 {
        0 // stub, 0 = success, required by Device supertrait
    }
}
