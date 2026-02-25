#![no_main]
#![no_std]
#![feature(decl_macro)]
#![feature(const_trait_impl)]
#![feature(const_default)]
#![feature(slice_ptr_get)]
#![feature(box_as_ptr)]
#![feature(const_range)]
#![feature(never_type)]
#![feature(sync_unsafe_cell)]
#![feature(custom_test_frameworks)]
#![test_runner(crate::test_runner)]
#![reexport_test_harness_main = "test_main"]

pub mod arch;
pub mod coroutine;
pub mod cmdline;
pub mod heap;
pub mod mp;
pub mod print;
pub mod thread;
pub mod sync;
pub mod local_storage;

extern crate alloc;


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

// some sample limine requests, for no particular reason
#[cfg(test)]
#[used]
#[unsafe(link_section = ".limine_requests")]
static BASE_REVISION: BaseRevision = BaseRevision::with_revision(4);

#[cfg(test)]
#[used]
#[unsafe(link_section = ".limine_requests")]
static BOOTLOADER_INFO_REQUEST: BootloaderInfoRequest = BootloaderInfoRequest::new();

#[cfg(test)]
#[used]
#[unsafe(link_section = ".limine_requests")]
static FIRMWARE_TYPE_REQUEST: FirmwareTypeRequest = FirmwareTypeRequest::new();

// ignore these
#[cfg(test)]
#[used]
#[unsafe(link_section = ".limine_requests_start")]
static _START_MARKER: RequestsStartMarker = RequestsStartMarker::new();

#[cfg(test)]
#[used]
#[unsafe(link_section = ".limine_requests_end")]
static _END_MARKER: RequestsEndMarker = RequestsEndMarker::new();

// heap
// TODO: use virtual memory herez
#[cfg(test)]
static mut THE_HEAP: [u8; 256 * 1024 * 1024] = [0; _];

#[cfg(test)]
fn dump_boot_info() {
    if let Some(res) = BOOTLOADER_INFO_REQUEST.get_response() {
        kprintln!("bootloader: {} v{}", res.name(), res.version());
    }

    if let Some(res) = get_cmdline_text() {
        kprintln!("cmdline: \"{}\"", res);
    }

    if let Some(err) = get_cmdline_error() {
        match err {
            cmdline::CmdlineError::NoResponse => {
                kprintln!("warn: no response received for cmdline request")
            }
            cmdline::CmdlineError::Utf8Error(err) => {
                kprintln!("warn: failed to convert cmdline to utf8: {}", err)
            }
            cmdline::CmdlineError::ParseError(err) => {
                kprintln!("warn: failed to parse cmdline: {}", err)
            }
        }
    }

    if let Some(res) = FIRMWARE_TYPE_REQUEST.get_response() {
        kprintln!(
            "firmware: {}",
            match res.firmware_type() {
                FirmwareType::X86_BIOS => "bios",
                FirmwareType::UEFI_32 => "efi_32",
                FirmwareType::UEFI_64 => "efi_64",
                FirmwareType::SBI => "sbi",
                _ => "unknown",
            }
        );
    }
}

#[cfg(test)]
static INIT_THREADING_BARRIER: Once<Barrier> = Once::new();
#[cfg(test)]
static MP_PREEMPT_ENTER_BARRIER: Once<Barrier> = Once::new();


#[cfg(test)]
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

    spawn_thread(move || {
        test_main();
    });

    irq_enable();
    poll_tasks();
}

#[cfg(test)]
unsafe extern "C" fn go_to_kernel_main() -> ! {
    kernel_main()
}

#[cfg(test)]
#[unsafe(no_mangle)]
unsafe extern "C" fn system_main() -> ! {
    assert!(BASE_REVISION.is_valid());

    parse_kernel_cmdline();
    init_tty();

    // print some system info
    if let Some(rev) = BASE_REVISION.loaded_revision() {
        kprintln!("limine rev: {}", rev);
    }

    if let Some(res) = BOOTLOADER_INFO_REQUEST.get_response() {
        kprintln!("bootloader: {} v{}", res.name(), res.version());
    }

    if let Some(res) = FIRMWARE_TYPE_REQUEST.get_response() {
        kprintln!(
            "fimrware: {}",
            match res.firmware_type() {
                FirmwareType::X86_BIOS => "bios",
                FirmwareType::UEFI_32 => "uefi (32-bit)",
                FirmwareType::UEFI_64 => "uefi (64-bit)",
                FirmwareType::SBI => "sbi",
                _ => "unknown",
            }
        )
    }

    init_malloc(Span::from_slice(&raw mut THE_HEAP));

    // note we don't need to do anything special here because rust doesn't have init_array
    // if we wanted once-initialized data, we would either provide our custom mechanism,
    // or just spam OnceCell

    // handle SSE/FSGSBASE/etc in initialize_mp
    initialize_mp(go_to_kernel_main);
}


// workaround for rust-analyzer being stupid
#[cfg(test)]
#[inline(always)]
#[allow(dead_code)]
fn rust_panic_impl(info: &core::panic::PanicInfo) -> ! {
    use crate::arch::halt;
    use crate::print::StackTrace;
    use crate::print::kprintln;

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

    #[cfg(test)]
    arch::shutdown(10 as u16);
    halt()
}

#[cfg(test)]
#[panic_handler]
fn rust_panic(info: &core::panic::PanicInfo) -> ! {
    rust_panic_impl(info);
}

// also copy-pasted from the tutorial
#[cfg(test)]
pub fn test_runner(tests: &'static [&(dyn Fn() + Send + Sync)]) {
    let x = alloc::sync::Arc::new(Barrier::new(tests.len()));
    for test in tests {
        test();
    }
    arch::shutdown(0);
}
