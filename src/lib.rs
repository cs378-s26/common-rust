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
pub mod cmdline;
pub mod coroutine;
pub mod heap;
pub mod local_storage;
pub mod mp;
pub mod physical_memory;
pub mod print;
pub mod sync;
pub mod test_utils;
pub mod thread;
pub mod virtual_memory;

extern crate alloc;
use crate::arch::{Arch, ArchTrait};
use crate::cmdline::parse_kernel_cmdline;
use crate::coroutine::{init_coroutine_executor, init_coroutine_queue};
use crate::heap::init_malloc;
use crate::mp::{MP_STAGE, MPStage, init_cpu_local_table};
use crate::print::{StackTrace, init_tty, kprintln};
use crate::thread::{poll_tasks, set_up_idle, spawn_thread};
use alloc::sync::Arc;
use core::sync::atomic::Ordering;
use limine::BaseRevision;
use limine::mp::Cpu;
use limine::firmware_type::FirmwareType;
use limine::request::{
    BootloaderInfoRequest, FirmwareTypeRequest, MpRequest, RequestsEndMarker, RequestsStartMarker,
};
use physical_memory::{THE_HEAP, init_physical_memory_allocator};
use spin::{Barrier, Once};
use talc::Span;
use virtual_memory::init_virtual_memory_allocator;

// some sample limine requests, for no particular reason
#[used]
#[unsafe(link_section = ".limine_requests")]
pub static BASE_REVISION: BaseRevision = BaseRevision::with_revision(4);

#[used]
#[unsafe(link_section = ".limine_requests")]
pub static BOOTLOADER_INFO_REQUEST: BootloaderInfoRequest = BootloaderInfoRequest::new();

#[used]
#[unsafe(link_section = ".limine_requests")]
pub static FIRMWARE_TYPE_REQUEST: FirmwareTypeRequest = FirmwareTypeRequest::new();

#[used]
#[unsafe(link_section = ".limine_requests")]
pub static MP_REQUEST: MpRequest = MpRequest::new();

#[used]
#[unsafe(link_section = ".limine_requests_start")]
pub static _START_MARKER: RequestsStartMarker = RequestsStartMarker::new();

#[used]
#[unsafe(link_section = ".limine_requests_end")]
pub static _END_MARKER: RequestsEndMarker = RequestsEndMarker::new();

pub trait KernelWorkTrait {
    fn work() -> ();
}

pub struct KernelWork;

impl KernelWorkTrait for KernelWork {
    fn work() {
        #[cfg(test)]
        test_main();
        Arch::shutdown(0);
    }
}

#[cfg(test)]
#[unsafe(no_mangle)]
unsafe extern "C" fn system_main() -> ! {
    system_init::<KernelWork>()
}

pub fn system_init<Work: KernelWorkTrait>() -> ! {
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
    init_physical_memory_allocator();
    init_virtual_memory_allocator();

    // note we don't need to do anything special here because rust doesn't have init_array
    // if we wanted once-initialized data, we would either provide our custom mechanism,
    // or just spam OnceCell

    // handle SSE/FSGSBASE/etc in initialize_mp
    let resp = MP_REQUEST
        .get_response()
        .expect("Expected to find MpResponse, found None.");
    init_cpu_local_table(resp.cpus().len());
    let mut bsp = None;
    let mut core_id: u64 = 1;
    for cpu in resp.cpus() {
        if Arch::is_bsp(&MP_REQUEST, cpu) {
            bsp = Some(cpu);
        } else {
            cpu.extra.store(core_id, Ordering::SeqCst);
            core_id += 1;
            cpu.goto_address.write(start_core::<Work>);
        }
    }
    unsafe { start_core::<Work>(bsp.expect("Couldn't find the bootstrap processor")) }
}

/// wrapper around initalize core that goes to kernel main
/// # Safety
/// Should only be called from bootstrap processor during kernel initialization
unsafe extern "C" fn start_core<Work: KernelWorkTrait>(cpu: &Cpu) -> ! {
    unsafe { Arch::initialize_core(cpu) };
    core_init::<Work>()
}

static INIT_THREADING_BARRIER: Once<Barrier> = Once::new();
static MP_PREEMPT_ENTER_BARRIER: Once<Barrier> = Once::new();
static MAKE_TEST_THREAD: Once<()> = Once::new();

pub fn core_init<Work: KernelWorkTrait>() -> ! {
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
            Work::work();
        })
    });

    Arch::set_irq_enabled(true);
    poll_tasks()
}

// also copy-pasted from the tutorial
pub fn test_runner(tests: &'static [&(dyn Fn() + Send + Sync)]) {
    let _barrier = Arc::new(Barrier::new(tests.len()));
    for test in tests {
        test();
    }
    Arch::shutdown(0);
}

#[cfg(test)]
#[panic_handler]
fn rust_panic(info: &core::panic::PanicInfo) -> ! {
    test_utils::rust_panic_impl(info);
}