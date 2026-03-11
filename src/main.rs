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

extern crate alloc;

use core::sync::atomic::Ordering;

// For coroutines.
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use kernel_common::arch::{Arch, ArchTrait, KernelEntryTrait, timer_ticks};
use kernel_common::coroutine::{init_coroutine_executor, init_coroutine_queue, spawn_coroutine};
use kernel_common::mp::{CORE_ID, MP_STAGE, MPStage};
use kernel_common::print::kprintln;
use kernel_common::thread::{init_threading, poll_tasks, set_up_idle, spawn_thread};
use spin::{Barrier, Once};

// For async/await testing. Move if/when we have a better testing setup.
struct IntFuture {
    value: u64,
    has_been_polled: bool,
}

impl Future for IntFuture {
    type Output = u64;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Randomly pend.
        if self.has_been_polled {
            Poll::Ready(self.value)
        } else {
            self.get_mut().has_been_polled = true;
            cx.waker().wake_by_ref(); // Theoretically, something else wakes this when ready.
            Poll::Pending
        }
    }
}

async fn async_int(number: u64) -> u64 {
    IntFuture {
        value: number,
        has_been_polled: false,
    }
    .await
}

async fn async_task(argument: u64) {
    for i in 0..4 {
        let n = async_int(i).await;
        kprintln!("Core {} async loop {}: {}", CORE_ID.get(), i, n);
    }
    kprintln!(
        "Core {} async task complete with argument: {}",
        CORE_ID.get(),
        argument
    );
}

struct MainKernelEntry;

impl KernelEntryTrait for MainKernelEntry {
    fn kernel_main() -> ! {
        kernel_main()
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn system_main() -> ! {
    kernel_common::system_init::<Arch, MainKernelEntry>()
}

static INIT_THREADING_BARRIER: Once<Barrier> = Once::new();
static MP_PREEMPT_ENTER_BARRIER: Once<Barrier> = Once::new();

pub fn kernel_main() -> ! {
    // kprintln!("we are the MPCorelings! please feed us!");
    let mp_res = kernel_common::MP_REQUEST
        .get_response()
        .expect("Expected to find MpResponse, found None.");
    let core_count = mp_res.cpus().len();

    INIT_THREADING_BARRIER
        .call_once(|| {
            kprintln!("preparing common tasks on {}", CORE_ID.get());
            kprintln!("there are {} cores total", core_count);
            init_threading();
            init_coroutine_queue();
            Barrier::new(core_count)
        })
        .wait();

    let idle = set_up_idle();

    kprintln!("init idle tid {} on core {}", idle.tid(), CORE_ID.get());

    init_coroutine_executor();
    kprintln!("Coroutine executor initialized.");

    MP_PREEMPT_ENTER_BARRIER
        .call_once(|| Barrier::new(core_count))
        .wait();

    MP_STAGE.store(MPStage::MPPreempt, Ordering::SeqCst);

    if CORE_ID.get().0 == 0 {
        spawn_coroutine(async_task(1624252));

        let num_threads = 20;
        kprintln!(
            "Spawning {} test threads across {} cores",
            num_threads,
            core_count
        );

        for i in 0..num_threads {
            spawn_thread(move || {
                let start_core = CORE_ID.get();
                let start_tick = timer_ticks();

                kprintln!(
                    "Thread {} started on core {} at tick {}",
                    i,
                    start_core.0,
                    start_tick
                );

                let cycle_start = Arch::read_cycle_counter();
                while Arch::read_cycle_counter() < cycle_start + 100_000_000 {}

                let end_core = CORE_ID.get();
                let end_tick = Arch::read_cycle_counter();

                kprintln!(
                    "Thread {} finished: started on core {}, ended on core {}, {} ticks elapsed",
                    i,
                    start_core.0,
                    end_core.0,
                    end_tick - start_tick,
                );
            });
        }
    }

    Arch::set_irq_enabled(true);
    poll_tasks()
}

// workaround for rust-analyzer being stupid
#[inline(always)]
#[allow(dead_code)]
fn rust_panic_impl(info: &core::panic::PanicInfo) -> ! {
    use kernel_common::arch::halt;
    use kernel_common::print::StackTrace;
    use kernel_common::print::kprintln;

    match info.location() {
        Some(location) => kprintln!(
            "panic: {}\nat {}:{}:{}\n{}",
            info.message(),
            location.file(),
            location.line(),
            location.column(),
            StackTrace::current()
        ),
        None => kprintln!(
            "panic: {}\nat unknown location\n{}",
            info.message(),
            StackTrace::current()
        ),
    };

    halt()
}

#[cfg(not(test))]
#[panic_handler]
fn rust_panic(info: &core::panic::PanicInfo) -> ! {
    rust_panic_impl(info);
}
