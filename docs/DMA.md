# DMA

This document describes the kernel's DMA (Direct Memory Access) support, which lives in `src/dma.rs`. DMA lets hardware devices read/write system memory directly without CPU involvement. Drivers need two things from the kernel to make this work: physically contiguous memory buffers with known physical addresses, and uncacheable mappings for device registers.

## Design Decisions

### DmaRegion uses HHDM, not a new virtual mapping

The kernel already has a Higher Half Direct Map (HHDM) that linearly maps all physical memory into virtual space. For any physical address `p`, the virtual address is just `p + HHDM_OFFSET`. So when we allocate contiguous physical frames for DMA, we get the virtual address for free — no need to create new page table entries or allocate virtual address space.

This is the same approach Linux uses (`page_to_virt` via the direct map).

### Cached memory is correct for DMA buffers on x86

On x86 with PCIe, DMA is cache-coherent — the hardware snoops CPU caches automatically. So DMA buffers use normal cached memory (via HHDM). No cache flushes needed. On ARM64 this also works for cache-coherent interconnects; non-coherent devices would need uncacheable buffers (future work).

### MMIO needs uncacheable mappings

Device registers (PCIe BARs) must be mapped as uncacheable to prevent the CPU from caching/reordering register reads and writes. `MmioRegion` creates a fresh virtual mapping with the `NO_CACHE` bit set (PCD on x86, DEVICE_MEMORY on ARM64). It uses `VirtualMemoryAllocation` with `owns_backing: false` so that dropping the mapping doesn't try to free hardware addresses as RAM.

## Interfaces

### DmaRegion — physically contiguous DMA buffer

```rust
use kernel_common::dma::DmaRegion;

// allocate 4 contiguous pages (16KB) for a ring buffer
let ring = DmaRegion::new(4);

// or allocate by size (rounds up to pages)
let buf = DmaRegion::new_bytes(8192);

// get addresses
let phys = ring.phys_addr(); // program this into the device
let virt = ring.virt_addr(); // CPU reads/writes here
let size = ring.size();      // in bytes

// read/write the buffer
let slice = ring.as_slice();              // &[u8]
let slice = unsafe { ring.as_slice_mut() }; // &mut [u8]
ring.zero();                              // clear to zeroes

// physical frames are returned to the allocator when `ring` goes out of scope
```

`as_slice_mut()` is unsafe because if a device is actively DMA-ing to the buffer, you have concurrent access. For setup/teardown when the device isn't active, this is fine.

### MmioRegion — device register mapping

```rust
use kernel_common::dma::MmioRegion;

// map a device's BAR (e.g. from PCI config space)
let regs = MmioRegion::new(bar_phys_addr, bar_size);

// volatile register access
let status: u32 = unsafe { regs.read::<u32>(0x04) };  // read status reg at offset 0x04
unsafe { regs.write::<u32>(0x00, 0x1) };               // write command reg at offset 0x00

// when `regs` goes out of scope, the virtual mapping is removed
// but the physical frames are NOT freed  they're hardware addresses, not RAM
```

All reads/writes are volatile. The mapping is uncacheable so register accesses go straight to the device.