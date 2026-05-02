#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_common::test_runner)]

extern crate alloc;

use alloc::{sync::Arc, vec::Vec};

use kernel_common::{
    arch::{Arch, ArchTrait},
    memory::virtual_memory::{PagingOptions, VirtualMemoryAllocation},
    print::kprintln,
    thread::spawn_thread,
};
use spin::{Barrier, Mutex};

kernel_common::integration_test!({
    fn rand(seed: &mut u8) -> u8 {
        // TODO use real OS pseudorandomness
        *seed = (*seed).wrapping_mul(37);
        *seed = (*seed).rotate_right(3);
        *seed ^= 0xA5;
        *seed
    }

    kprintln!("virtual memory patterns test started");
    const ITERATIONS: usize = 64;
    let mut seed = 0xed;
    let mut vmas = Vec::new();
    for _ in 0..ITERATIONS {
        if vmas.is_empty() || rand(&mut seed).is_multiple_of(2) {
            let vma = VirtualMemoryAllocation::new(
                Arch::get_kernel_address_space(),
                None,
                Arch::PAGE_SIZE * rand(&mut seed) as usize,
                None,
                PagingOptions::PRESENT
                    | PagingOptions::WRITABLE
                    | PagingOptions::CACHEABLE
                    | PagingOptions::GLOBAL,
                true,
            )
            .unwrap();
            for j in (0..vma.length).step_by(Arch::PAGE_SIZE) {
                // write to every page in the allocation
                unsafe { *((vma.base + j) as *mut u8) = j as u8 | 0x80 };
            }
            vmas.push(vma);
        } else {
            let vma = vmas.remove((rand(&mut seed) as usize) % vmas.len());
            for j in (0..vma.length).step_by(Arch::PAGE_SIZE) {
                // check every page in the allocation
                assert!(unsafe { *((vma.base + j) as *mut u8) } == j as u8 | 0x80);
            }
        }
    }
    while let Some(vma) = vmas.pop() {
        for j in (0..vma.length).step_by(Arch::PAGE_SIZE) {
            // check every page in the allocation
            assert!(unsafe { *((vma.base + j) as *mut u8) } == j as u8 | 0x80);
        }
    }
    kprintln!("virtual memory patterns test completed");

    // TODO! why is this only dealloc'ing one VA?
    kprintln!("virtual memory threading test started");
    const THREADS: usize = 8;
    let thread_barrier: Arc<Barrier> = Arc::new(Barrier::new(THREADS));
    let test_barrier: Arc<Barrier> = Arc::new(Barrier::new(THREADS + 1));
    let bases: Arc<Mutex<Vec<VirtualMemoryAllocation>>> = Arc::new(Mutex::new(Vec::new()));
    for _ in 0..THREADS {
        let thread_barrier = thread_barrier.clone();
        let test_barrier = test_barrier.clone();
        let thread_bases = bases.clone();
        spawn_thread(move || {
            for i in 0..16 {
                let size = Arch::PAGE_SIZE * (i + 1);
                let mmapped = VirtualMemoryAllocation::new(
                    Arch::get_kernel_address_space(),
                    None,
                    size,
                    None,
                    PagingOptions::PRESENT
                        | PagingOptions::WRITABLE
                        | PagingOptions::CACHEABLE
                        | PagingOptions::GLOBAL,
                    true,
                )
                .unwrap(); // allocations of increasing sizes
                for j in (0..size).step_by(Arch::PAGE_SIZE) {
                    // write to every page in the allocation
                    unsafe { *((mmapped.base + j) as *mut u8) = j as u8 };
                }
                let mut lock = thread_bases.lock();
                (*lock).push(mmapped);
                drop(lock);
                (*thread_barrier).wait();
                for t in 0..THREADS {
                    let lock = thread_bases.lock();
                    let vma = lock[t].base;
                    drop(lock);
                    for j in (0..size).step_by(Arch::PAGE_SIZE) {
                        // read from every page in every allocation
                        assert!(unsafe { *((vma + j) as *mut u8) } == j as u8);
                    }
                }
                (*thread_barrier).wait();
                let mut lock = thread_bases.lock();
                lock.pop();
                drop(lock);
                (*thread_barrier).wait();
            }
            (*test_barrier).wait();
        });
    }
    (*test_barrier).wait();
    kprintln!("virtual memory threading test complete");
});
