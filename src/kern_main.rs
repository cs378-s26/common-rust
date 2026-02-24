
use core::arch::asm;
use core::sync::atomic::Ordering;

// For coroutines.
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use limine::BaseRevision;
use limine::firmware_type::FirmwareType;
use limine::request::{
    BootloaderInfoRequest, FirmwareTypeRequest, RequestsEndMarker, RequestsStartMarker,
};
use spin::{Barrier, Once};
use talc::Span;
use x86::time::rdtsc;

use crate::arch::{core_count, initialize_mp, irq_enable};
use crate::coroutine::{init_coroutine_executor, init_coroutine_queue, spawn_coroutine};
use crate::cmdline::{get_cmdline_error, get_cmdline_text, parse_kernel_cmdline};
use crate::heap::init_malloc;
use crate::mp::{CORE_ID, MP_STAGE, MPStage};
use crate::print::{init_tty, kprintln};
use crate::thread::{Thread, init_threading, poll_tasks, set_up_idle, spawn_thread, yield_thread};

static INIT_THREADING_BARRIER: Once<Barrier> = Once::new();
static MP_PREEMPT_ENTER_BARRIER: Once<Barrier> = Once::new();

pub fn kernel_main() -> ! {
    // kprintln!("we are the MPCorelings! please feed us!");

    INIT_THREADING_BARRIER
        .call_once(|| {
            kprintln!("hii~");
            kprintln!("preparing common tasks on {}", CORE_ID.get());
            kprintln!("there are {} cores total", core_count());
            init_threading();
            init_coroutine_queue();
            Barrier::new(core_count())
        })
        .wait();

    let idle = set_up_idle();

    kprintln!("init tid: core={}, {}", CORE_ID.get(), idle.tid());

    init_coroutine_executor();
    kprintln!("Coroutine executor initialized.");

    MP_PREEMPT_ENTER_BARRIER
        .call_once(|| Barrier::new(core_count()))
        .wait();

    MP_STAGE.store(MPStage::MPPreempt, Ordering::SeqCst);

    let initial_core = CORE_ID.get();

    for i in 0..1000 {
        spawn_thread(move || {
            kprintln!("hi, id={}, initial_core={}", i, initial_core);

            // bad sleep function :D
            let tsc = unsafe { rdtsc() };
            while unsafe { rdtsc() } < tsc + 10000000000 {
                yield_thread();
            }

            kprintln!(
                "meow from {}, id={}, initial_core={}, tid={}",
                CORE_ID.get(),
                i,
                initial_core,
                Thread::this_tid()
            );
            loop {
                yield_thread();
            }
        });
    }

    irq_enable();
    poll_tasks();
}
