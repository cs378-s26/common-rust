#![no_std]
#![no_main]
#![feature(decl_macro)]
#![feature(const_trait_impl)]
#![feature(const_default)]
#![feature(slice_ptr_get)]
#![feature(box_as_ptr)]
#![feature(const_range)]

extern crate alloc;

mod arch;
mod heap;
mod local_storage;
mod mp;
mod print;
mod sync;
mod thread;

use core::sync::atomic::Ordering;

use limine::BaseRevision;
use limine::firmware_type::FirmwareType;
use limine::request::{
    BootloaderInfoRequest, FirmwareTypeRequest, RequestsEndMarker, RequestsStartMarker,
};
use spin::{Barrier, Once};
use talc::Span;

use crate::arch::{core_count, halt, initialize_mp, irq_enable};
use crate::heap::init_malloc;
use crate::mp::{CORE_ID, MP_STAGE, MPStage};
use crate::print::{init_tty, kprintln};
use crate::thread::{init_threading, poll_tasks, set_up_idle, spawn_thread, yield_thread};

// some sample limine requests, for no particular reason
#[used]
#[unsafe(link_section = ".limine_requests")]
static BASE_REVISION: BaseRevision = BaseRevision::with_revision(4);

#[used]
#[unsafe(link_section = ".limine_requests")]
static BOOTLOADER_INFO_REQUEST: BootloaderInfoRequest = BootloaderInfoRequest::new();

#[used]
#[unsafe(link_section = ".limine_requests")]
static FIRMWARE_TYPE_REQUEST: FirmwareTypeRequest = FirmwareTypeRequest::new();

// ignore these
#[used]
#[unsafe(link_section = ".limine_requests_start")]
static _START_MARKER: RequestsStartMarker = RequestsStartMarker::new();

#[used]
#[unsafe(link_section = ".limine_requests_end")]
static _END_MARKER: RequestsEndMarker = RequestsEndMarker::new();

// heap
// TODO: use virtual memory herez
static mut THE_HEAP: [u8; 128 * 1024 * 1024] = [0; _];

#[unsafe(no_mangle)]
unsafe extern "C" fn system_main() -> ! {
    assert!(BASE_REVISION.is_valid());

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
    initialize_mp();
}

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
            Barrier::new(core_count())
        })
        .wait();

    set_up_idle();

    MP_PREEMPT_ENTER_BARRIER
        .call_once(|| Barrier::new(core_count()))
        .wait();

    MP_STAGE.store(MPStage::MPPreempt, Ordering::SeqCst);

    spawn_thread(|| {
        kprintln!("meow from {}", CORE_ID.get());
        loop {
            yield_thread();
        }
    });

    irq_enable();
    poll_tasks();
}

// workaround for rust-analyzer being stupid
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

    halt()
}

#[cfg(not(test))]
#[panic_handler]
fn rust_panic(info: &core::panic::PanicInfo) -> ! {
    rust_panic_impl(info);
}
