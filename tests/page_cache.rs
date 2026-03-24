#![no_std]
#![no_main]
#![feature(decl_macro)]
#![feature(const_trait_impl)]
#![feature(const_default)]
#![feature(slice_ptr_get)]
#![feature(box_as_ptr)]
#![feature(const_range)]
#![feature(never_type)]
#![feature(sync_unsafe_cell)]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_common::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;

use core::sync::atomic::{AtomicUsize, Ordering};

use alloc::sync::Arc;
use kernel_common::MP_REQUEST;
use kernel_common::arch::{Arch, ArchTrait, KernelEntryTrait};
use kernel_common::coroutine::{init_coroutine_executor, init_coroutine_queue};
use kernel_common::event::init_event_handler;
use kernel_common::mp::{MP_STAGE, MPStage};
use kernel_common::page_cache::VirtualMemory;
use kernel_common::print::kprintln;
use kernel_common::process::Process;
use kernel_common::ramfs::RAMFilesystem;
use kernel_common::thread::{THIS_THREAD, poll_tasks, set_up_idle, spawn_thread};
use kernel_common::vfs::{INodeKey, VFS};
use spin::{Barrier, Mutex, Once};

#[cfg(test)]
static INIT_THREADING_BARRIER: Once<Barrier> = Once::new();
#[cfg(test)]
static MP_PREEMPT_ENTER_BARRIER: Once<Barrier> = Once::new();
#[cfg(test)]
static MAKE_TEST_THREAD: Once<()> = Once::new();
#[cfg(test)]
pub struct TestKernelEntry;
#[cfg(test)]
impl KernelEntryTrait for TestKernelEntry {
    fn kernel_main() -> ! {
        let mp_res = MP_REQUEST
            .get_response()
            .expect("Expected to find MpResponse, found None.");
        let core_count = mp_res.cpus().len();

        INIT_THREADING_BARRIER
            .call_once(|| {
                init_coroutine_queue();
                Barrier::new(core_count)
            })
            .wait();

        set_up_idle();
        init_event_handler();

        init_coroutine_executor();
        kprintln!("Coroutine executor initialized.");

        MP_PREEMPT_ENTER_BARRIER
            .call_once(|| Barrier::new(core_count))
            .wait();

        MP_STAGE.store(MPStage::MPPreempt, Ordering::SeqCst);

        MAKE_TEST_THREAD.call_once(|| {
            spawn_thread(move || {
                kprintln!("Starting Testing Code...");
                crate::test_main();
            })
        });

        Arch::set_irq_enabled(true);
        poll_tasks()
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn system_main() -> ! {
    kernel_common::system_init::<Arch, TestKernelEntry>();
}

#[panic_handler]
fn rust_panic(info: &core::panic::PanicInfo) -> ! {
    kernel_common::test_utils::rust_panic_test_impl(info);
}

static LATCH: AtomicUsize = AtomicUsize::new(0);

fn init() {
    VFS.mount(RAMFilesystem::new());
}

#[test_case]
fn cow() {
    init();
    LATCH.fetch_add(1, Ordering::SeqCst);
    let process = Arc::new(Process::new());
    Process::run_process(process, || {
        let x;
        let y;
        {
            let thread = THIS_THREAD.get().unwrap().upgrade().unwrap();
            let process = thread.process.get().unwrap();
            x = process
                .virtual_memory
                .mmap(Some((INodeKey::new(0, 1), 0, None)), 4096, true, None)
                .unwrap() as *mut u8;
            y = process
                .virtual_memory
                .mmap(Some((INodeKey::new(0, 1), 0, None)), 4096, false, None)
                .unwrap() as *mut u8;
        }
        assert!(x != y);
        unsafe {
            assert!(*x == b'c');
            assert!(*y == b'c');
            *x = b'b';
            assert!(*y == b'b');
            *y = b'd';
            assert!(*x == b'b');
            *x = b'c';
        }
        kprintln!("Cow successful");
        LATCH.fetch_sub(1, Ordering::SeqCst);
    });
    while LATCH.load(Ordering::SeqCst) > 0 {}
}

#[test_case]
fn large_anon() {
    init();
    LATCH.fetch_add(1, Ordering::SeqCst);
    let process = Arc::new(Process::new());
    Process::run_process(process, || {
        let x;
        {
            let thread = THIS_THREAD.get().unwrap().upgrade().unwrap();
            let process = thread.process.get().unwrap();
            x = process
                .virtual_memory
                .mmap(None, 4096 * 100000, false, None)
                .unwrap();
        }
        unsafe {
            for i in 0..100 {
                *((x + 4096 * i * 1000) as *mut u8) = 1;
            }
            for i in 0..100 {
                assert!(*((x + 4096 * i * 1000) as *const u8) == 1);
            }
        }
        kprintln!("Large anon successful");
        LATCH.fetch_sub(1, Ordering::SeqCst);
    });
    while LATCH.load(Ordering::SeqCst) > 0 {}
}

// TODO: write a better partial case using the dog file
#[test_case]
fn partial() {
    init();
    LATCH.fetch_add(1, Ordering::SeqCst);
    let process = Arc::new(Process::new());
    Process::run_process(process, || {
        let x;
        let y;
        {
            let thread = THIS_THREAD.get().unwrap().upgrade().unwrap();
            let process = thread.process.get().unwrap();
            x = process
                .virtual_memory
                .mmap(Some((INodeKey::new(0, 1), 0, None)), 4096, true, None)
                .unwrap() as *mut u8;
            y = process
                .virtual_memory
                .mmap(Some((INodeKey::new(0, 1), 0, Some(2))), 4096, false, None)
                .unwrap() as *mut u8;
        }
        assert!(x != y);
        unsafe {
            assert!(*x == b'c');
            assert!(*(x.add(1)) == b'a');
            assert!(*(x.add(2)) == b't');

            assert!(*(y.add(2)) == 0);
            assert!(*(y.add(1)) == b'a');
            assert!(*y == b'c');
        }
        kprintln!("Partial successful");
        LATCH.fetch_sub(1, Ordering::SeqCst);
    });
    while LATCH.load(Ordering::SeqCst) > 0 {}
}

#[test_case]
fn interprocess() {
    init();
    LATCH.fetch_add(3, Ordering::SeqCst);
    let process = Arc::new(Process::new());
    Process::run_process(process, || {
        let file_shared;
        let file_private;
        let anon_shared;
        let anon_private;
        let new_process;
        {
            let thread = THIS_THREAD.get().unwrap().upgrade().unwrap();
            let process = thread.process.get().unwrap();
            file_shared = process
                .virtual_memory
                .mmap(Some((INodeKey::new(0, 2), 0, None)), 4096, true, None)
                .unwrap();
            file_private = process
                .virtual_memory
                .mmap(Some((INodeKey::new(0, 2), 0, None)), 4096, false, None)
                .unwrap();
            anon_shared = process.virtual_memory.mmap(None, 4096, true, None).unwrap();
            anon_private = process
                .virtual_memory
                .mmap(None, 4096, false, None)
                .unwrap();
            new_process = Arc::new(Process::clone(process));
        }

        unsafe {
            *(anon_shared as *mut u8) = b'x';
            *(anon_private as *mut u8) = b'x';
            *(file_shared as *mut u8) = b'x';
            *(file_private as *mut u8) = b'x';
        }
        Process::run_process(new_process, move || {
            unsafe {
                *(anon_shared as *mut u8) = b'y';
                *(anon_private as *mut u8) = b'y';
                *(file_shared as *mut u8) = b'y';
                *(file_private as *mut u8) = b'y';
            }
            LATCH.fetch_sub(1, Ordering::SeqCst);
            unsafe {
                assert!(*(anon_shared as *mut u8) == b'y');
                assert!(*(anon_private as *mut u8) == b'y');
                assert!(*(anon_shared as *mut u8) == b'y');
                assert!(*(anon_private as *mut u8) == b'y');
            }
            LATCH.fetch_sub(1, Ordering::SeqCst);
        });
        while LATCH.load(Ordering::SeqCst) > 2 {}
        unsafe {
            assert!(*(anon_shared as *mut u8) == b'y');
            assert!(*(anon_private as *mut u8) == b'x');
            assert!(*(file_shared as *mut u8) == b'y');
            assert!(*(file_private as *mut u8) == b'x');
            *(file_shared as *mut u8) = b'c';
        }
        kprintln!("Interprocess successful");
        LATCH.fetch_sub(1, Ordering::SeqCst);
    });
    while LATCH.load(Ordering::SeqCst) > 0 {}
}
