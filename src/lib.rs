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
pub mod devices;
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
use crate::arch::{Arch, ArchTrait, KernelEntryTrait};
use crate::cmdline::parse_kernel_cmdline;
use crate::heap::init_malloc;
use crate::mp::init_cpu_local_table;
use crate::print::{StackTrace, init_tty, kprintln};
use alloc::sync::Arc;
use limine::BaseRevision;
use limine::firmware_type::FirmwareType;
use limine::request::{
    BootloaderInfoRequest, FirmwareTypeRequest, MpRequest, RequestsEndMarker, RequestsStartMarker,
};
use physical_memory::{THE_HEAP, init_physical_memory_allocator};
use spin::Barrier;
use talc::Span;
use virtual_memory::init_virtual_memory_allocator;

#[cfg(test)]
mod test {
    use super::{Arch, ArchTrait, KernelEntryTrait, MP_REQUEST};
    use crate::coroutine::{init_coroutine_executor, init_coroutine_queue};
    use crate::mp::{MP_STAGE, MPStage};
    use crate::print::kprintln;
    use crate::test_utils;
    use crate::thread::{poll_tasks, set_up_idle, spawn_thread};
    use core::sync::atomic::Ordering;
    use spin::{Barrier, Once};

    static INIT_THREADING_BARRIER: Once<Barrier> = Once::new();
    static MP_PREEMPT_ENTER_BARRIER: Once<Barrier> = Once::new();
    static MAKE_TEST_THREAD: Once<()> = Once::new();

    pub struct TestKernelEntry;

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
                kprintln!("Starting Testing Code...");
                spawn_thread(move || {
                    crate::test_main();
                })
            });

            Arch::set_irq_enabled(true);
            poll_tasks()
        }
    }

    #[unsafe(no_mangle)]
    unsafe extern "C" fn system_main() -> ! {
        crate::system_init::<Arch, TestKernelEntry>();
    }

    #[panic_handler]
    fn rust_panic(info: &core::panic::PanicInfo) -> ! {
        test_utils::rust_panic_test_impl(info);
    }
}

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

pub fn system_init<A: ArchTrait, K: KernelEntryTrait>() -> ! {
    assert!(BASE_REVISION.is_valid());

    parse_kernel_cmdline();
    init_malloc(Span::from_slice(&raw mut THE_HEAP));
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

    init_physical_memory_allocator();
    kprintln!("here");
    init_virtual_memory_allocator();
    kprintln!("vmem");

    Arch::create_arch_specific_drivers();
    Arch::parse_devices();
    Arch::init_arch_specific_drivers();

    // note we don't need to do anything special here because rust doesn't have init_array
    // if we wanted once-initialized data, we would either provide our custom mechanism,
    // or just spam OnceCell

    // handle SSE/FSGSBASE/etc in initialize_mp
    let mp_res = MP_REQUEST
        .get_response()
        .expect("Expected to find MpResponse, found None.");
    init_cpu_local_table(mp_res.cpus().len());
    A::initialize_mp::<K>(&MP_REQUEST)
}

// also copy-pasted from the tutorial
pub fn test_runner(tests: &'static [&(dyn Fn() + Send + Sync)]) {
    let _barrier = Arc::new(Barrier::new(tests.len()));
    for test in tests {
        test();
    }
    Arch::shutdown(0);
}
