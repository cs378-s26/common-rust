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

## Design and Tradeoffs

The kernel system currently uses two complementary intrusive red-black trees to manage virtual memory: one for tracking free virtual address ranges, sorted primarily by length to enable efficient search for a suitably large region (and in fact the smallest such region, giving us best-fit allocation essentially for free) and another for tracking allocated regions, sorted by starting address to enable efficient deallocation, coalescing, and page fault handling.

This data structure is initialized during the boot sequence in `system_init`, after a call to an architecture-specific function to configure virtual memory. 

Currently, this system does not support multiple address spaces, so the `space` parameter is effectively ignored, but we include it for future extensibility (e.g., for kernel threads that wish to override the default kernel mappings with their own, and to potentially allow this system to be extended to serve user mappings).

We support up to level-4 page tables on x86-64, giving us 48 bits of virtual address space. This matches Limine's default page table setup as well as the `x86_64` crate's default configuration.