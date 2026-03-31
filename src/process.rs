use alloc::sync::Arc;

use crate::{
    arch::{Arch, ArchTrait},
    thread::{THIS_THREAD, spawn_thread},
    virtual_memory_2::VirtualMemory,
};

pub struct Process {
    pub virtual_memory: VirtualMemory,
}

impl Process {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            virtual_memory: VirtualMemory::new(),
        })
    }

    pub fn fork(&self) -> Arc<Self> {
        Arc::new(Self {
            virtual_memory: self.virtual_memory.clone(),
        })
    }

    pub fn run<T: FnOnce() + Send + 'static>(process: Arc<Self>, task: T) {
        spawn_thread(move || {
            let thread = THIS_THREAD.get().unwrap().upgrade().unwrap();
            let process = thread.process.call_once(|| Arc::clone(&process));
            Arch::set_user_address_space(process.virtual_memory.get_page_table() as u64);
            task()
        });
    }
}

#[cfg(test)]
mod test {
    use crate::{
        arch::{Arch, ArchTrait},
        physical_memory::{frame_alloc, frame_dealloc},
        process::Process,
        thread::yield_thread,
        virtual_memory::PagingOptions,
        virtual_memory_2::VirtualMemory,
    };

    #[test_case]
    fn test_processes() {
        const VADDR: usize = 0x10000;
        for i in 0..128 {
            let process = Process::new();
            Process::run(process.clone(), move || {
                let paddr = frame_alloc();
                assert!(
                    process.virtual_memory.get_page_table()
                        != VirtualMemory::get_limine_page_table()
                );
                Arch::virtual_map(
                    process.virtual_memory.get_page_table() as u64,
                    VADDR as u64,
                    paddr as u64,
                    PagingOptions::PRESENT | PagingOptions::WRITABLE | PagingOptions::CACHEABLE,
                );
                unsafe {
                    *(VADDR as *mut u8) = i;
                    yield_thread();
                    assert!(*(VADDR as *const u8) == i);
                }
                Arch::virtual_unmap(process.virtual_memory.get_page_table() as u64, VADDR as u64);
                frame_dealloc(paddr);
            });
        }
    }
}
