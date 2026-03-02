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
pub mod print;
pub mod sync;
pub mod thread;
pub mod physical_memory;
pub mod virtual_memory;

extern crate alloc;

#[cfg(test)]
mod test_runtime {
    use alloc::sync::Arc;
    use core::sync::atomic::Ordering;

    use limine::BaseRevision;
    use limine::firmware_type::FirmwareType;
    use limine::request::{
        BootloaderInfoRequest, FirmwareTypeRequest, MpRequest, RequestsEndMarker,
        RequestsStartMarker,
    };
    use spin::{Barrier, Once};
    use talc::Span;

    use crate::arch::{Arch, ArchTrait, KernelEntryTrait};
    use crate::cmdline::{self, get_cmdline_error, get_cmdline_text, parse_kernel_cmdline};
    use crate::coroutine::{init_coroutine_executor, init_coroutine_queue};
    use crate::heap::init_malloc;
    use crate::mp::{CORE_ID, MP_STAGE, MPStage, init_cpu_local_table};
    use crate::physical_memory::{init_physical_memory_allocator};
    use crate::print::{StackTrace, init_tty, kprintln};
    use crate::thread::{init_threading, poll_tasks, set_up_idle, spawn_thread};
    use crate::virtual_memory::init_virtual_memory_allocator;

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

    #[used]
    #[unsafe(link_section = ".limine_requests")]
    static MP_REQUEST: MpRequest = MpRequest::new();

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

    static INIT_THREADING_BARRIER: Once<Barrier> = Once::new();
    static MP_PREEMPT_ENTER_BARRIER: Once<Barrier> = Once::new();
    static MAKE_TEST_THREAD: Once<()> = Once::new();

    struct TestKernelEntry;

    impl KernelEntryTrait for TestKernelEntry {
        fn kernel_main() -> ! {
            kernel_main()
        }
    }

    pub fn kernel_main() -> ! {
        // kprintln!("we are the MPCorelings! please feed us!");
        let mp_res = MP_REQUEST
            .get_response()
            .expect("Expected to find MpResponse, found None.");
        let core_count = mp_res.cpus().len();

        INIT_THREADING_BARRIER
            .call_once(|| {
                kprintln!("hii~");
                kprintln!("preparing common tasks on {}", CORE_ID.get());
                kprintln!("there are {} cores total", core_count);
                init_threading();
                init_coroutine_queue();
                Barrier::new(core_count)
            })
            .wait();

        let idle = set_up_idle();

        kprintln!("init tid: core={}, {}", CORE_ID.get(), idle.tid());

        init_coroutine_executor();
        kprintln!("Coroutine executor initialized.");

        MP_PREEMPT_ENTER_BARRIER
            .call_once(|| Barrier::new(core_count))
            .wait();

        MP_STAGE.store(MPStage::MPPreempt, Ordering::SeqCst);

        MAKE_TEST_THREAD
            .call_once(|| {
                spawn_thread(move || {
                    crate::test_main();
                })
            });

        Arch::set_irq_enabled(true);
        kprintln!("Polling for tasks...");
        poll_tasks();
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

        init_physical_memory_allocator();
        init_virtual_memory_allocator();

        // handle SSE/FSGSBASE/etc in initialize_mp
        let mp_res = MP_REQUEST
            .get_response()
            .expect("Expected to find MpResponse, found None.");
        init_cpu_local_table(mp_res.cpus().len());
        Arch::initialize_mp::<TestKernelEntry>(&MP_REQUEST)
    }

    // workaround for rust-analyzer being stupid
    #[inline(always)]
    #[allow(dead_code)]
    fn rust_panic_impl(info: &core::panic::PanicInfo) -> ! {
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

        Arch::shutdown(10 as u16);
        Arch::halt()
    }

    #[panic_handler]
    fn rust_panic(info: &core::panic::PanicInfo) -> ! {
        rust_panic_impl(info);
    }

    // also copy-pasted from the tutorial
    pub fn test_runner(tests: &'static [&(dyn Fn() + Send + Sync)]) {
        let _barrier = Arc::new(Barrier::new(tests.len()));
        kprintln!("Running tests...");
        for test in tests {
            test();
        }
        kprintln!("Done running tests.");
        Arch::shutdown(0);
    }
}

#[cfg(test)]
pub use test_runtime::{kernel_main, test_runner};
