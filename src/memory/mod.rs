pub mod dma;
pub mod freeset;
pub mod heap;
pub mod page_cache;
pub mod physical_memory;
pub mod virtual_memory;
pub mod virtual_memory_2;

use physical_memory::EarlyPmm;
use virtual_memory::init_virtual_memory_allocator;
use virtual_memory_2::VirtualMemory;

pub fn init_mm() {
    let mut early = EarlyPmm::init();
    let pages = init_virtual_memory_allocator(&mut early);
    physical_memory::init(early, pages);
    VirtualMemory::init();
}
