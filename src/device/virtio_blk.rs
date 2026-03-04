extern crate virtio_drivers;
use core::ptr::NonNull;
use virtio_drivers::transport::{Transport, mmio::MmioTransport};
use virtio_drivers::{Hal, device::blk, BufferDirection, PhysAddr};
use kernel_common::virtual_memory::{VirtualMemoryAllocation, PagingOptions};
use kernel_common::physical_memory::{HHDM_REQUEST}
use kernel_common::arch::{Arch, ArchTrait};
use spin::Once; 

struct VirtioBlk<H: Hal, T: Transport> {
    blk: blk::VirtIOBlk<H, T>,
}

struct VirtioHal;

unsafe impl Hal for VirtioHal {
    fn dma_alloc(pages: usize, _direction: BufferDirection) -> (PhysAddr, NonNull<u8>) {
        let hhdm = HHDM_REQUEST.get_response().unwrap().offset() as usize;

        // TODO currently ignoring direction and setting it to be non cacheable by the cpu, but maybe we want to change this
        let options = PagingOptions::PRESENT | PagingOptions::WRITABLE;
        let vaddr = VirtualMemoryAllocation::new(Arch::get_address_space(), Arch::PAGE_SIZE * pages, None, options);
        let paddr = 
    
    }
    
    unsafe fn dma_dealloc(paddr: virtio_drivers::PhysAddr, vaddr: NonNull<u8>, pages: usize) -> i32 {
        todo!()
    }
    
    unsafe fn mmio_phys_to_virt(paddr: virtio_drivers::PhysAddr, size: usize) -> NonNull<u8> {
        todo!()
    }
    
    unsafe fn share(buffer: NonNull<[u8]>, direction: virtio_drivers::BufferDirection) -> virtio_drivers::PhysAddr {
        todo!()
    }
    
    unsafe fn unshare(paddr: virtio_drivers::PhysAddr, buffer: NonNull<[u8]>, direction: virtio_drivers::BufferDirection) {
        todo!()
    }
    
}

pub fn init_virtio_blk(base_addr: usize, size: usize) {
    let transport = MmioTransport::new(NonNull::new(base_addr as *mut u8).unwrap(), size);
    let blk_device = virtio_drivers::device::blk::VirtIOBlk::new(transport).unwrap();

    
}