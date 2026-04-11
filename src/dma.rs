use crate::{
    arch::{Arch, ArchTrait},
    physical_memory::{HHDM_OFFSET, alloc_frames, frame_dealloc},
    virtual_memory::{PagingOptions, VirtualMemoryAllocation},
};
use core::ptr;

pub struct DmaRegion {
    phys_addr: usize,
    virt_addr: usize,
    num_frames: usize,
}

impl DmaRegion {
    /// Allocate `num_frames` physically contiguous 4KB pages for DMA.
    pub fn new(num_frames: usize) -> DmaRegion {
        let phys_addr = alloc_frames(num_frames);
        let virt_addr = phys_addr + *HHDM_OFFSET.get().unwrap();

        unsafe {
            ptr::write_bytes(virt_addr as *mut u8, 0, num_frames * Arch::PAGE_SIZE);
        }

        DmaRegion {
            phys_addr,
            virt_addr,
            num_frames,
        }
    }

    pub fn new_bytes(size: usize) -> DmaRegion {
        let num_frames = size.div_ceil(Arch::PAGE_SIZE);
        Self::new(num_frames)
    }

    /// Physical base address (for programming into device registers).
    pub fn phys_addr(&self) -> usize {
        self.phys_addr
    }

    /// Virtual base address (for CPU access).
    pub fn virt_addr(&self) -> usize {
        self.virt_addr
    }

    /// Size in bytes.
    pub fn size(&self) -> usize {
        self.num_frames * Arch::PAGE_SIZE
    }

    /// Get a read-only slice of the entire DMA region.
    pub fn as_slice(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.virt_addr as *const u8, self.size()) }
    }

    /// Get a mutable slice of the entire DMA region.
    /// # Safety
    /// Caller must ensure no concurrent device access to the same range,
    /// or that such concurrent access is acceptable for the use case.
    pub unsafe fn as_slice_mut(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.virt_addr as *mut u8, self.size()) }
    }

    /// Zero the entire DMA region.
    pub fn zero(&self) {
        unsafe {
            ptr::write_bytes(self.virt_addr as *mut u8, 0, self.size());
        }
    }
}

impl Drop for DmaRegion {
    fn drop(&mut self) {
        for i in 0..self.num_frames {
            frame_dealloc(self.phys_addr + i * Arch::PAGE_SIZE);
        }
    }
}

/// A mapped MMIO region for device register access.
/// Maps physical device memory (e.g. PCIe BARs) as uncacheable. Provides
/// volatile read/write accessors for register access. On drop, unmaps the
/// virtual pages without freeing physical frames (they belong to hardware).
pub struct MmioRegion {
    phys_addr: usize,
    virt_addr: usize,
    size: usize,
    _vma: VirtualMemoryAllocation,
}

impl MmioRegion {
    /// Map `size` bytes of device MMIO space starting at `phys_addr`.
    /// The region is mapped as uncacheable (PCD=1 on x86, DEVICE_MEMORY on ARM).
    pub fn new(phys_addr: usize, size: usize) -> MmioRegion {
        let aligned_size = (size + Arch::PAGE_SIZE - 1) & !(Arch::PAGE_SIZE - 1);

        let vma = VirtualMemoryAllocation::new(
            Arch::get_kernel_address_space(),
            None,
            aligned_size,
            Some(phys_addr),
            PagingOptions::PRESENT | PagingOptions::WRITABLE | PagingOptions::DEVICE_MEMORY, // deliberately no CACHEABLE
            false, // don't free physical frames on drop — they're hardware registers
        )
        .expect("failed to allocate virtual address range for MMIO");

        MmioRegion {
            phys_addr,
            virt_addr: vma.base,
            size: aligned_size,
            _vma: vma,
        }
    }

    /// Read a value at byte offset from the MMIO base.
    /// # Safety
    /// Caller must ensure the offset is valid for the device and properly aligned for T.
    pub unsafe fn read<T: Copy>(&self, offset: usize) -> T {
        unsafe { ptr::read_volatile((self.virt_addr + offset) as *const T) }
    }

    /// Write a value at byte offset from the MMIO base.
    /// # Safety
    /// Caller must ensure the offset is valid for the device and properly aligned for T.
    pub unsafe fn write<T: Copy>(&self, offset: usize, value: T) {
        unsafe { ptr::write_volatile((self.virt_addr + offset) as *mut T, value) }
    }

    pub fn phys_addr(&self) -> usize {
        self.phys_addr
    }

    pub fn virt_addr(&self) -> usize {
        self.virt_addr
    }

    pub fn size(&self) -> usize {
        self.size
    }
}

#[cfg(test)]
mod test {
    use crate::dma::DmaRegion;
    use crate::physical_memory::HHDM_OFFSET;
    use crate::print::kprintln;
    use crate::thread::{spawn_thread, yield_thread};
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicI64, Ordering};

    #[test_case]
    fn test_dma_region() {
        let barrier = Arc::new(AtomicI64::new(1));
        let b = barrier.clone();
        spawn_thread(move || {
            kprintln!("=== DMA region tests ===");

            // basic allocation
            kprintln!("allocating 1-frame DMA region");
            let mut region = DmaRegion::new(1);
            assert!(region.size() == 4096);
            //          There is an address at physical address 0!
            //            assert!(region.phys_addr() != 0);
            assert!(region.virt_addr() == region.phys_addr() + *HHDM_OFFSET.get().unwrap());

            // zeroed on allocation
            kprintln!("verifying DMA region is zeroed");
            for &byte in region.as_slice() {
                assert!(byte == 0);
            }

            // read/write via slice
            kprintln!("testing DMA region read/write");
            unsafe {
                let buf = region.as_slice_mut();
                for (i, item) in buf.iter_mut().enumerate() {
                    *item = (i & 0xFF) as u8;
                }
            }
            let buf = region.as_slice();
            for (i, item) in buf.iter().enumerate() {
                assert!(*item == (i & 0xFF) as u8);
            }

            // zero
            kprintln!("testing DMA region zero");
            region.zero();
            for &byte in region.as_slice() {
                assert!(byte == 0);
            }

            // drop should not panic
            kprintln!("testing DMA region drop");
            drop(region);

            // multi-frame allocation
            kprintln!("allocating 4-frame DMA region");
            let mut region = DmaRegion::new(4);
            assert!(region.size() == 4096 * 4);

            // verify contiguous read/write across pages
            kprintln!("verifying multi-frame DMA region");
            unsafe {
                let buf = region.as_slice_mut();
                for (i, item) in buf.iter_mut().enumerate() {
                    *item = (i & 0xFF) as u8;
                }
            }
            for i in 0..region.size() {
                assert!(region.as_slice()[i] == (i & 0xFF) as u8);
            }
            drop(region);

            // new_bytes rounds up correctly
            kprintln!("testing DMA new_bytes rounding");
            let region = DmaRegion::new_bytes(5000);
            assert!(region.size() == 4096 * 2);
            drop(region);

            kprintln!("=== DMA region tests passed ===");
            (*b).fetch_add(-1, Ordering::SeqCst);
            loop {
                yield_thread();
            }
        });
        while (*barrier).load(Ordering::SeqCst) > 0 {
            yield_thread();
        }
    }
}
