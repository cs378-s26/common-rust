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

use alloc::sync::Arc;
use core::sync::atomic::{AtomicU64, Ordering};
use kernel_common::MP_REQUEST;
use kernel_common::arch::{Arch, ArchTrait, KernelEntryTrait};
use kernel_common::coroutine::{init_coroutine_executor, init_coroutine_queue};
use kernel_common::event::init_event_handler;
use kernel_common::mp::{MP_STAGE, MPStage};
use kernel_common::print::kprintln;
use kernel_common::process::Process;
use kernel_common::ramfs::RAMFilesystem;
use kernel_common::thread::{poll_tasks, set_up_idle, spawn_thread};
use kernel_common::vfs::{INodeKey, VFS};
use spin::{Barrier, Once};

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

static LATCH: AtomicU64 = AtomicU64::new(0);

fn test01() {
    LATCH.fetch_add(1, Ordering::SeqCst);
    let process = Process::new();
    Process::run(process.clone(), move || {
        let x = process
            .virtual_memory
            .mmap(
                Some((INodeKey::new(0, 1), 0, None)),
                Arch::PAGE_SIZE,
                true,
                None,
            )
            .unwrap();
        let y = process
            .virtual_memory
            .mmap(
                Some((INodeKey::new(0, 1), 0, None)),
                Arch::PAGE_SIZE,
                true,
                None,
            )
            .unwrap();
        assert!(x != y);
        unsafe {
            assert!(*(y as *const u8) == b'c');
            *(x as *mut u8) = b'b';
            assert!(*(y as *const u8) == b'b');
            *(y as *mut u8) = b'a';
            assert!(*(x as *const u8) == b'a');
            *(x as *mut u8) = b'c';
        };
        LATCH.fetch_sub(1, Ordering::SeqCst);
    });
    while LATCH.load(Ordering::SeqCst) > 0 {}
}

fn test02() {
    LATCH.fetch_add(1, Ordering::SeqCst);
    let process = Process::new();
    Process::run(process.clone(), move || {
        let x = process
            .virtual_memory
            .mmap(
                Some((INodeKey::new(0, 2), 0, None)),
                Arch::PAGE_SIZE,
                true,
                None,
            )
            .unwrap();
        let y = process
            .virtual_memory
            .mmap(
                Some((INodeKey::new(0, 2), 0, Some(Arch::PAGE_SIZE + 2))),
                Arch::PAGE_SIZE * 3,
                false,
                None,
            )
            .unwrap();
        assert!(x != y);
        unsafe {
            // COW for first page
            assert!(*(y as *const u8) == b'd');
            *(x as *mut u8) = b'b';
            assert!(*(y as *const u8) == b'b');
            *(y as *mut u8) = b'l';
            assert!(*(x as *const u8) == b'b');
            *(x as *mut u8) = b'd';

            // COR for second page
            assert!(*((y + Arch::PAGE_SIZE) as *const u8) == b'o');
            *((x + Arch::PAGE_SIZE) as *mut u8) = b'l';
            assert!(*((y + Arch::PAGE_SIZE) as *const u8) == b'o');
            *((y + Arch::PAGE_SIZE) as *mut u8) = b'm';
            assert!(*((x + Arch::PAGE_SIZE) as *const u8) == b'l');
            *((x + Arch::PAGE_SIZE) as *mut u8) = b'o';
            for i in 2..Arch::PAGE_SIZE {
                assert!(*((y + Arch::PAGE_SIZE + i) as *const u8) == 0);
            }

            // Blank third page
            for i in 0..Arch::PAGE_SIZE {
                assert!(*((y + 2 * Arch::PAGE_SIZE + i) as *const u8) == 0);
            }
        };
        LATCH.fetch_sub(1, Ordering::SeqCst);
    });
    while LATCH.load(Ordering::SeqCst) > 0 {}
}

fn test03() {
    LATCH.fetch_add(1, Ordering::SeqCst);
    let process = Process::new();
    Process::run(process.clone(), move || {
        let x = process
            .virtual_memory
            .mmap(None, Arch::PAGE_SIZE, false, None)
            .unwrap();
        unsafe {
            *(x as *mut u8) = b'd';
            *((x + 1) as *mut u8) = b'o';
            *((x + 2) as *mut u8) = b'g';

            assert!(*(x as *const u8) == b'd');
            assert!(*((x + 1) as *const u8) == b'o');
            assert!(*((x + 2) as *const u8) == b'g');
            for i in 3..Arch::PAGE_SIZE {
                assert!(*((x + i) as *const u8) == 0);
            }
        };
        LATCH.fetch_sub(1, Ordering::SeqCst);
    });
    while LATCH.load(Ordering::SeqCst) > 0 {}
}

fn test04() {
    LATCH.fetch_add(3, Ordering::SeqCst);
    let process = Process::new();
    Process::run(process.clone(), move || {
        let file_shared = process
            .virtual_memory
            .mmap(Some((INodeKey::new(0, 1), 0, None)), 4096, true, None)
            .unwrap();
        let file_private = process
            .virtual_memory
            .mmap(Some((INodeKey::new(0, 1), 0, None)), 4096, false, None)
            .unwrap();
        let anon_shared = process.virtual_memory.mmap(None, 4096, true, None).unwrap();
        let anon_private = process
            .virtual_memory
            .mmap(None, 4096, false, None)
            .unwrap();
        let new_process = process.fork();
        unsafe {
            *(anon_shared as *mut u8) = b'x';
            *(anon_private as *mut u8) = b'x';
            *(file_shared as *mut u8) = b'x';
            *(file_private as *mut u8) = b'x';
        }
        Process::run(new_process, move || {
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
        }
        while LATCH.load(Ordering::SeqCst) > 1 {}
        unsafe { *(file_private as *mut u8) = b'c' };
        LATCH.fetch_sub(1, Ordering::SeqCst);
    });
    while LATCH.load(Ordering::SeqCst) > 0 {}
}

#[test_case]
fn run() {
    VFS.mount(RAMFilesystem::new());
    test01();
    test02();
    test03();
    test04();
}
