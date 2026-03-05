extern crate virtio_drivers;
use core::ptr::NonNull;
use kernel_common::arch::{Arch, ArchTrait};
use kernel_common::physical_memory::{HHDM_REQUEST, frame_alloc, alloc_frames, frame_dealloc};
use kernel_common::print::kprintln;
use kernel_common::virtual_memory::PagingOptions;
use spin::Once;
use virtio_drivers::transport::{Transport, mmio::MmioTransport, mmio::VirtIOHeader};
use virtio_drivers::{BufferDirection, Hal, PhysAddr, device::blk};

struct VirtioBlk<H: Hal, T: Transport> {
    blk: blk::VirtIOBlk<H, T>,
}

struct VirtioBlkHal;

// necessary struct for virtio driver to communicate with hardware.
unsafe impl Hal for VirtioBlkHal {
    fn dma_alloc(pages: usize, _direction: BufferDirection) -> (PhysAddr, NonNull<u8>) {
        kprintln!("Allocating dma buffer");
        let hhdm = HHDM_REQUEST.get_response().unwrap().offset() as usize;

        // TODO currently ignoring direction and setting it to be non cacheable by the cpu, but maybe we want to change this
        let options = PagingOptions::PRESENT | PagingOptions::WRITABLE;
        let paddr = alloc_frames(pages) as u64;
        let vaddr = paddr + hhdm as u64;
        // This is already mapped, we are doing this simply to change it to be mapped as device memory
        // to prevent caching issues. This is mapped back to normal later
        // TODO get previous pt mapping so it can be reinstalled later properly
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
        let nn = NonNull::new(vaddr as *mut u8).unwrap();
        (paddr, nn)
    }

    unsafe fn dma_dealloc(paddr: PhysAddr, vaddr: NonNull<u8>, pages: usize) -> i32 {
        // TODO see prev todo
        kprintln!("Deallocating dma buffer");
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
        return 0;
    }

    // maps a physical mmio region to a virtual address, must be mapped
    unsafe fn mmio_phys_to_virt(paddr: virtio_drivers::PhysAddr, size: usize) -> NonNull<u8> {
        kprintln!("mapping mmio");
        let hhdm = HHDM_REQUEST.get_response().unwrap().offset() as usize;

        // get the total amount of pages covered by the region and
        let page_offset = (paddr as usize) % Arch::PAGE_SIZE;
        let pages_covered = (page_offset + size + Arch::PAGE_SIZE - 1) / Arch::PAGE_SIZE;

        for page in 0..pages_covered {
            Arch::virtual_map(
                Arch::get_address_space(),
                (paddr as usize + page * Arch::PAGE_SIZE + hhdm) as u64,
                (paddr as usize + page * Arch::PAGE_SIZE) as u64,
                PagingOptions::PRESENT | PagingOptions::WRITABLE,
            );
        }
        let nn = NonNull::new((paddr as usize + hhdm) as *mut u8).unwrap();
        nn
    }

    unsafe fn share(buffer: NonNull<[u8]>, _direction: BufferDirection) -> PhysAddr {
        kprintln!("Sharing buffer with virtio device");
        let pages = (buffer.len() + Arch::PAGE_SIZE - 1) / Arch::PAGE_SIZE;
        let (paddr, _) = Self::dma_alloc(pages, _direction);
        unsafe {
            core::ptr::copy_nonoverlapping(
                buffer.as_ptr() as *const u8,
                (paddr + HHDM_REQUEST.get_response().unwrap().offset() as u64) as *mut u8,
                buffer.len(),
            );
        }
        return paddr;
    }

    unsafe fn unshare(
        paddr: virtio_drivers::PhysAddr,
        buffer: NonNull<[u8]>,
        _direction: virtio_drivers::BufferDirection,
    ) {
        kprintln!("Unsharing buffer with virtio device");
        let pages = (buffer.len() + Arch::PAGE_SIZE - 1) / Arch::PAGE_SIZE;
        unsafe {
            core::ptr::copy_nonoverlapping(
                (paddr + HHDM_REQUEST.get_response().unwrap().offset() as u64) as *const u8,
                buffer.as_ptr() as *mut u8,
                buffer.len(),
            );
            Self::dma_dealloc(paddr, NonNull::new(buffer.as_ptr() as *mut u8).unwrap(), pages);
        }
    }
}

pub fn init_virtio_blk(base_addr: usize, size: usize) {
    unsafe {
        let transport =
            MmioTransport::new(NonNull::new(base_addr as *mut VirtIOHeader).unwrap(), size)
                .unwrap();
        let mut blk_device =
            virtio_drivers::device::blk::VirtIOBlk::<VirtioBlkHal, MmioTransport>::new(transport)
                .unwrap();
        kprintln!(
            "virtio blk device initialized with capacity {} bytes",
            blk_device.capacity() * 512
        );
        let mut buf = [0u8; 512];
        // let phys_addr = frame_alloc();
        // let hhdm = HHDM_REQUEST.get_response().unwrap().offset() as usize;
        // let mut buf = core::slice::from_raw_parts_mut((phys_addr + hhdm) as *mut u8, 512);
        
        kprintln!("buf len: {}", buf.len());
        buf[0] = 42;
        blk_device.write_blocks(0, &mut buf).unwrap();
        kprintln!("finished writing");
        blk_device.read_blocks(0, &mut buf).unwrap();
        kprintln!("Read from virtio blk device: {}", buf[0]);
    }
}
