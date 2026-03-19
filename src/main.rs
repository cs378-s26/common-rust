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

use core::sync::atomic::{AtomicU64, Ordering};

// For coroutines.
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use kernel_common::arch::{Arch, ArchTrait, timer_ticks};
use kernel_common::coroutine::{init_coroutine_executor, init_coroutine_queue, spawn_coroutine};
use kernel_common::mp::{CORE_ID, MP_STAGE, MPStage};
use kernel_common::print::kprintln;
use kernel_common::thread::{poll_tasks, set_up_idle, spawn_thread};
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
