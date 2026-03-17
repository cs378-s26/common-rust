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

use kernel_common::MP_REQUEST;
use kernel_common::arch::{Arch, ArchTrait, KernelEntryTrait};
use kernel_common::coroutine::{init_coroutine_executor, init_coroutine_queue};
use kernel_common::mp::{CORE_ID, MP_STAGE, MPStage};
use kernel_common::print::kprintln;
use kernel_common::state::{CorePin, StateGuard};
use kernel_common::sync::{IntMutex, MutexLike};
use kernel_common::thread::{poll_tasks, set_up_idle, spawn_thread, yield_thread};
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

#[cfg(test)]
fn work(i: u64) -> u64 {
    let mut sum = 0;
    for j in 1..(1 << i) {
        sum += j;
    }
    sum
}

const THREADS: usize = 16;
static LATCH: AtomicUsize = AtomicUsize::new(0);
const UPPER: usize = 21; // precisely tuned value for runtime
static VALUES: IntMutex<[u64; UPPER]> = IntMutex::new([0; UPPER]);

#[test_case]
fn hello_world() {
    // stress preemption, yielding, and blocking interactions
    kprintln!("spawning busyworking threads");
    for _ in 0..THREADS {
        spawn_thread(|| {
            for i in 3..UPPER {
                let _guard = if i.is_multiple_of(2) {
                    Some(StateGuard::<CorePin>::guard())
                } else {
                    None
                };
                let core = CORE_ID.get();
                let value = if i.is_multiple_of(3) {
                    let lock = VALUES.lock();
                    let val = work(i as u64);
                    drop(lock);
                    val
                } else {
                    work(i as u64)
                };
                if i.is_multiple_of(2) {
                    assert!(core == CORE_ID.get());
                }
                let mut lock = VALUES.lock();
                if lock[i] == 0 {
                    lock[i] = value;
                    kprintln!("work({}): {}", i, value); // deliberately inside lock to try to encourage blocking
                }
                drop(lock);
                yield_thread();
            }
            LATCH.fetch_add(1, Ordering::SeqCst);
        });
        yield_thread();
    }
    while LATCH.load(Ordering::SeqCst) != THREADS {} // spin
    LATCH.store(0, Ordering::SeqCst);

    // are we actually preempting?
    kprintln!("spawning infinite loop threads");
    for i in 0..THREADS {
        spawn_thread(|| {
            loop {
                LATCH.fetch_add(1, Ordering::SeqCst);
                let value = work(48); // should never actually finish
                kprintln!("{}", value); // appease linter
            }
        });
        if i == 0 {
            kprintln!("spawned first thread")
        };
        yield_thread();
        if i == 0 {
            kprintln!("returned from yielding to first thread")
        };
    }
    while LATCH.load(Ordering::SeqCst) != THREADS {} // spin
    kprintln!("yay we seem to have actually preempted");

    kprintln!("test complete!");
    Arch::shutdown(0);
}
