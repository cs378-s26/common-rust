# Virtual Memory

## Kernel Mapping Interface

Our kernel-space virtual memory interface uses Rust's type system to manage allocations, returning an object (containing relevant information about the mapping) that is automatically deallocated when dropped (unless explicity indicated otherwise by the `PagingOptions::SHADOW` flag).

We take six explicit parameters, as listed below.

```rust
space: u64,             // address space identifier
start: Option<usize>,   // fixed mapping location
length: usize,          // requested size in bytes
backing: Option<usize>, // physical frames used
options: PagingOptions, // similar to mmap flags
owns_backing: bool,     // if false, Drop won't free physical frames 
```

The PagingOptions parameter serves as a sort of catch-all for specifying various roughly binary features of the mapping.

```rust
pub struct PagingOptions: u64 {
    const PRESENT = 1 << 0;
    const WRITABLE = 1 << 1;
    const EXECUTABLE = 1 << 2;
    const USER_ACCESSIBLE = 1 << 3;
    const WRITE_THROUGH = 1 << 4;
    const CACHEABLE = 1 << 5;
    const GLOBAL = 1 << 6;
    const FIXED = 1 << 7;
    const SHADOW = 1 << 8;
    const DEVICE_MEMORY = 1 << 9;
}
```

## Architecture-Specific Interfaces

`get_{kernel, user}_address_space() -> u64` - returns the current address space identifier. Should not have side effects.

`set_user_address_space(cr3: u64)` - should set the current user address space to the one identified by `cr3`. The parameter should be renamed to be architecture-agnostic. This should come with any side-effects necessary to ensure that the new address space is active (e.g., flushing the TLB on x86-64, or performing data synchronization operations on ARM).

`virtual_map(space: u64, vaddr: u64, paddr: u64, options: PagingOptions)` - should install the given virtual-to-physical mapping in the page tables (correctly updating their structure by allocating intermediate-level page tables as necessary) for the given address space, with the specified options (given in the same format as for the overall virtual memory allocator). 

`virtual_unmap(space: u64, vaddr: u64)` - should remove the virtual-to-physical mapping for the given address space at the given virtual address, optionally freeing the associated physical frame.

`virtual_unmap_no_dealloc(space: u64, vaddr: u64)` - same as above, but doesn't deallocate the physical frame at that mapping.

`fn virtual_invalidate(vaddr: u64)` - should invalidate the TLB entry for the given virtual address on the current CPU. Necessary whenever mappings are changed to avoid using stale mappings or mappings settings (e.g., when unmapping pages or changing a read-only COW page to be writable).

`shootdown_tlbs(space: u64, base: usize, length: usize)` - called to invalidate the . Issues an *event* (see `event.rs`) for `x86-64` and triggers an interrupt to cause all CPUs to switch to handling that event. On ARM, this can be implemented by simply issuing the appropriate instruction and performing the necessary data synchronization operations, so this is effectively a no-op.

## Design and Tradeoffs

The kernel system currently uses two complementary intrusive red-black trees to manage virtual memory: one for tracking free virtual address ranges, sorted primarily by length to enable efficient search for a suitably large region (and in fact the smallest such region, giving us best-fit allocation essentially for free) and another for tracking allocated regions, sorted by starting address to enable efficient deallocation, coalescing, and page fault handling.

This data structure is initialized during the boot sequence in `system_init`, after a call to an architecture-specific function to configure virtual memory. 

Currently, this system does not support multiple address spaces, so the `space` parameter is effectively ignored, but we include it for future extensibility (e.g., for kernel threads that wish to override the default kernel mappings with their own, and to potentially allow this system to be extended to serve user mappings).

We support up to level-4 page tables on x86-64, giving us 48 bits of virtual address space. This matches Limine's default page table setup as well as the `x86_64` crate's default configuration.

To avoid inserting architecture-specific code into the core virtual memory management logic, we call architecture-specific functions for allocating and freeing physical frames, modifying page table entries, flushing the TLB, and issuing shootdowns. However, some aspects of these interfaces admittedly leave some things to be desired. For example, TLB shootdowns are not actually needed on ARM (we just need a single instruction and four data synchronization operations).

We currently do not distinguish between address spaces when handling shootdowns, which is quite inefficient both in terms of stray invalidations and wasted time. Perhaps a better interface would be to have the architecture-specific unmapping function handle this, which would require changing it to accept a range of pages to unmap in order to avoid interrupt thrashing.