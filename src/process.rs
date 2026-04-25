use alloc::sync::Arc;
extern crate bitvec;
use bitvec::prelude::{BitVec, bitvec};
use spin::Once;

use crate::{
    arch::{Arch, ArchTrait},
    memory::virtual_memory_2::VirtualMemory,
    sync::{IntMutex, MutexLike, Promise},
    thread::{THIS_THREAD, spawn_thread},
};

static MAX_PID: usize = 65536;
static PID_ALLOCATOR: Once<IntMutex<PidAllocator>> = Once::new();

struct PidAllocator {
    used: BitVec,
    next_hint: usize,
}

impl PidAllocator {
    fn new() -> Self {
        // +1 so index == pid works for 0..=MAX_PID
        let mut used = bitvec![0; MAX_PID + 1];

        // Reserve PID 0
        used.set(0, true);

        Self { used, next_hint: 1 }
    }

    fn alloc(&mut self) -> Option<u32> {
        // First scan from next_hint upward
        for pid in self.next_hint..=MAX_PID {
            if !self.used[pid] {
                self.used.set(pid, true);
                self.next_hint = if pid == MAX_PID { 1 } else { pid + 1 };
                return Some(pid as u32);
            }
        }

        // Then wrap around and scan from 1 up to next_hint - 1
        for pid in 1..self.next_hint {
            if !self.used[pid] {
                self.used.set(pid, true);
                self.next_hint = if pid == MAX_PID { 1 } else { pid + 1 };
                return Some(pid as u32);
            }
        }

        None
    }

    // TODO implement proper process cleanup and call this
    fn _free(&mut self, pid: u32) {
        let pid = pid as usize;
        assert!(pid >= 1 && pid <= MAX_PID);
        assert!(self.used[pid], "double free of PID {}", pid);
        self.used.set(pid, false);
    }

}

pub fn init_pid_allocator() {
    PID_ALLOCATOR.call_once(|| IntMutex::new(PidAllocator::new()));
}

pub struct Process {
    pub virtual_memory: VirtualMemory,
    pub exit_code: Promise<i32>,
    pid: u32,
}

impl Process {
    pub fn new() -> Option<Arc<Self>> {
        let pid = PID_ALLOCATOR.get().unwrap().lock().alloc()?;
        Some(Arc::new(Self {
            virtual_memory: VirtualMemory::new(),
            exit_code: Promise::new(),
            pid,
        }))
    }

    pub fn run<T: FnOnce() + Send + 'static>(process: Arc<Self>, task: T) {
        spawn_thread(move || {
            let thread = THIS_THREAD.get().unwrap().upgrade().unwrap();
            let process = thread.process.call_once(|| Arc::clone(&process));
            Arch::set_user_address_space(process.virtual_memory.get_page_table() as u64);
            task()
        });
    }
    pub fn get_pid(&self) -> u32 {
        self.pid
    }
}

#[cfg(test)]
mod test {
    use crate::{
        arch::{Arch, ArchTrait},
        memory::{
            physical_memory::frame_alloc, virtual_memory::PagingOptions,
            virtual_memory_2::VirtualMemory,
        },
        process::Process,
        thread::yield_thread,
    };

    #[test_case]
    fn test_processes() {
        const VADDR: usize = 0x80000000;
        for i in 0..128 {
            let process = Process::new().expect("failed to create process");
            Process::run(process.clone(), move || {
                let paddr = frame_alloc();
                assert!(
                    process.virtual_memory.get_page_table()
                        == Arch::get_user_address_space() as usize
                );
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
            });
        }
    }
}
