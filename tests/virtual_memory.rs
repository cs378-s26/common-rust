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

use core::sync::atomic::Ordering;
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

fn test01() {
    let process = Process::new();
    Process::run(process.clone(), move || {
        let x = process
            .virtual_memory
            .mmap(Some((INodeKey::new(0, 1), 0, None)), 4096, true, None)
            .unwrap();
        let y = process
            .virtual_memory
            .mmap(Some((INodeKey::new(0, 1), 0, None)), 4096, true, None)
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
    });
}

fn test02() {
    let process = Process::new();
    Process::run(process.clone(), move || {
        let x = process
            .virtual_memory
            .mmap(Some((INodeKey::new(0, 1), 0, None)), 4096, true, None)
            .unwrap();
        let y = process
            .virtual_memory
            .mmap(Some((INodeKey::new(0, 1), 0, Some(2))), 4096, true, None)
            .unwrap();
        assert!(x != y);
        unsafe {
            assert!(*(y as *const u8) == b'c');
            *(x as *mut u8) = b'b';
            assert!(*(y as *const u8) == b'c');
            *(y as *mut u8) = b'a';
            assert!(*(x as *const u8) == b'b');
            *(x as *mut u8) = b'c';
            assert!(*((x + 2) as *const u8) == b't');
            assert!(*((y + 2) as *const u8) == 0);
        };
    });
}

fn test03() {
    let process = Process::new();
    Process::run(process.clone(), move || {
        let x = process
            .virtual_memory
            .mmap(None, 4096, false, None)
            .unwrap();
        unsafe {
            *(x as *mut u8) = b'd';
            *((x + 1) as *mut u8) = b'o';
            *((x + 2) as *mut u8) = b'g';

            assert!(*(x as *const u8) == b'd');
            assert!(*((x + 1) as *const u8) == b'o');
            assert!(*((x + 2) as *const u8) == b'g');
        };
    });
}

#[test_case]
fn run() {
    VFS.mount(RAMFilesystem::new());
    test01();
    test02();
    test03();
}
