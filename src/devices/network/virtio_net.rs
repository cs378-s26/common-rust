extern crate virtio_drivers;
use super::{NetworkDevice, NetworkError};
use crate::arch::{Arch, ArchTrait};
use crate::devices::Device;
use crate::physical_memory::{HHDM_REQUEST, alloc_frames, frame_dealloc};
use crate::virtual_memory::{PagingOptions, VirtualMemoryAllocation};
use core::ptr::NonNull;
use virtio_drivers::device::net::{TxBuffer, VirtIONet};
use virtio_drivers::transport::Transport;
use virtio_drivers::{BufferDirection, Hal, PhysAddr};

/// VirtIONetDriver wraps the virtio-drivers VirtIONet device and ties it to our kernel's HAL
/// constructed from a transport (MMIO) and used by the device framework to send and receive packets
pub struct VirtIONetDriver<H: Hal, T: Transport, const QUEUE_SIZE: usize> {
    net: VirtIONet<H, T, QUEUE_SIZE>,
}

impl<T: Transport> VirtIONetDriver<VirtioNetHal, T, 16> {
    pub fn new(transport: T) -> Self {
        Self {
            net: VirtIONet::<VirtioNetHal, T, 16>::new(transport, 1536)
                .expect("failed to initialize virtio net device"),
        }
    }
}

/// VirtioNetHal provides the kernel-side implementation of the virtio Hal trait for the net driver
/// the virtio-drivers crate is hardware-agnostic, it calls into this to allocate DMA memory and
/// map MMIO regions Mirrors VirtioBlkHal in src/devices/block/virtio_blk.rs
pub struct VirtioNetHal;

// necessary struct for virtio net driver to communicate with hardware
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
        0 // 0 = success required by Hal trait (C-style status code)
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

    // allocates a DMA bounce buffer and copies data into it if the direction is driver-to-device
    // returns the physical address for the device to read from 
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

impl<T: Transport> NetworkDevice for VirtIONetDriver<VirtioNetHal, T, 16> {
    fn name(&self) -> &str {
        "virtio_net"
    }

    fn send_packet(&mut self, packet: &[u8]) -> Result<(), NetworkError> {
        self.net
            .send(TxBuffer::from(packet))
            .map_err(|_| NetworkError::SendError)
    }

    fn receive_packet(&mut self, buffer: &mut [u8]) -> Result<usize, NetworkError> {
        let rx_buf = self.net.receive().map_err(|_| NetworkError::ReceiveError)?;
        let packet = rx_buf.packet();
        let len = packet.len().min(buffer.len());
        buffer[..len].copy_from_slice(&packet[..len]);
        self.net
            .recycle_rx_buffer(rx_buf)
            .map_err(|_| NetworkError::ReceiveError)?;
        Ok(len)
    }
}

impl<T: Transport> Device for VirtIONetDriver<VirtioNetHal, T, 16> {
    #[allow(unused_variables)]
    fn ioctl(&self, request: u64, arg1: u64, arg2: u64) -> u64 {
        0 // stub 0 = success required by Device supertrait
    }
}
