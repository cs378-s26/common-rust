use crate::{
    arch::{Arch, ArchTrait},
    page_cache::VirtualMemory,
    thread::{THIS_THREAD, spawn_thread},
};
use alloc::sync::Arc;

pub struct Process {
    pub virtual_memory: VirtualMemory,
}

// TODO: Clean up page tables

impl Process {
    pub fn new() -> Self {
        Self {
            virtual_memory: VirtualMemory::new(),
        }
    }

    pub fn spawn_thread<T: FnOnce() + Send + 'static>(task: T) {
        spawn_thread(|| {
            {
                let thread = THIS_THREAD.get().unwrap().upgrade().unwrap();
                let process = thread.process.call_once(|| Arc::new(Process::new()));
                Arch::set_address_space(process.virtual_memory.get_page_table() as u64);
            }
            task()
        });
    }
}
