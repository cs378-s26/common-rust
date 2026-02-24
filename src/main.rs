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

// For coroutines.
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use kernel_common::*;

use limine::BaseRevision;
use limine::firmware_type::FirmwareType;
use limine::request::{
    BootloaderInfoRequest, FirmwareTypeRequest, RequestsEndMarker, RequestsStartMarker,
};
use spin::{Barrier, Once};
use talc::Span;
use x86::time::rdtsc;

use kernel_common::arch::{core_count, initialize_mp, irq_enable};
use kernel_common::coroutine::{init_coroutine_executor, init_coroutine_queue, spawn_coroutine};
use kernel_common::cmdline::{get_cmdline_error, get_cmdline_text, parse_kernel_cmdline};
use kernel_common::heap::init_malloc;
use kernel_common::mp::{CORE_ID, MP_STAGE, MPStage};
use kernel_common::print::{init_tty, kprintln};
use kernel_common::thread::{Thread, init_threading, poll_tasks, set_up_idle, spawn_thread, yield_thread};

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
static mut THE_HEAP: [u8; 256 * 1024 * 1024] = [0; _];

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
    initialize_mp();
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