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
pub mod elf;
pub mod event;
pub mod fs;
pub mod local_storage;
pub mod memory;
pub mod modules;
pub mod mp;
pub mod panic;
pub mod print;
pub mod process;
pub mod state;
pub mod symbols;
pub mod sync;
pub mod syscall;
pub mod thread;
extern crate alloc;
use alloc::sync::Arc;
use core::sync::atomic::Ordering;

use limine::{
    BaseRevision,
    firmware_type::FirmwareType,
    mp::Cpu,
    request::{
        BootloaderInfoRequest, FirmwareTypeRequest, MpRequest, RequestsEndMarker,
        RequestsStartMarker,
    },
};
use memory::{
    physical_memory::{THE_HEAP, init_physical_memory_allocator},
    virtual_memory::init_virtual_memory_allocator,
};
use modules::load_modules_early;
use spin::{Barrier, Once};

use crate::{
    arch::{Arch, ArchTrait},
    cmdline::{get_cmdline_error, get_cmdline_text, parse_kernel_cmdline},
    coroutine::{init_coroutine_executor, init_coroutine_queue},
    devices::discovery::{create_drivers, discover_devices},
    event::init_event_handler,
    fs::{
        fake::{FAKE, Fake},
        vfs::VFS,
    },
    memory::{heap::init_malloc, virtual_memory_2::VirtualMemory},
    mp::{MP_STAGE, MPStage, init_cpu_local_table},
    print::{StackTrace, init_tty, kprintln},
    process::init_pid_allocator,
    thread::{poll_tasks, set_up_idle, spawn_thread},
};

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
        #[cfg(not(test))]
        kprintln!("entered kernel");
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
    init_malloc((&raw mut THE_HEAP) as usize, 256 * 1024 * 1024);
    init_tty();

    load_modules_early();

    // print some system info
    if let Some(rev) = BASE_REVISION.loaded_revision() {
        kprintln!("limine rev: {}", rev);
    }

    if let Some(res) = BOOTLOADER_INFO_REQUEST.get_response() {
        kprintln!("bootloader: {} v{}", res.name(), res.version());
    }

    if let Some(err) = get_cmdline_error() {
        match err {
            cmdline::CmdlineError::NoResponse => {
                kprintln!("no response received for cmdline request")
            }
            cmdline::CmdlineError::Utf8Error(err) => {
                kprintln!("failed to convert cmdline to utf8: {}", err)
            }
            cmdline::CmdlineError::ParseError(err) => kprintln!("failed to parse cmdline: {}", err),
        }
    }

    if let Some(res) = get_cmdline_text() {
        kprintln!("cmdline: \"{}\"", res);
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
    init_virtual_memory_allocator();
    init_pid_allocator();

    VirtualMemory::init();
    let fake = Arc::clone(FAKE.call_once(Fake::new));
    VFS.mount(fake);

    create_drivers();
    kprintln!("First round of device discovery...");
    discover_devices(true);
    kprintln!("Finished first round of device discovery.");

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
            cpu.goto_address.write(core_init::<Work>);
        }
    }
    unsafe { core_init::<Work>(bsp.expect("Couldn't find the bootstrap processor")) }
}

/// wrapper around initalize core that goes to kernel main
/// # Safety
/// Should only be called from bootstrap processor during kernel initialization
unsafe extern "C" fn core_init<Work: KernelWorkTrait>(cpu: &Cpu) -> ! {
    unsafe { Arch::initialize_core(cpu) };
    let mp_res = MP_REQUEST
        .get_response()
        .expect("Expected to find MpResponse, found None.");
    let core_count = mp_res.cpus().len();

    // runs an initialization routine once overall
    // waits for this to complete before any core proceeds
    macro one($code:block) {{
        // needs to be in an extra block to avoid namespace collisions
        static BARRIER: Once<Barrier> = Once::new();
        BARRIER
            .call_once(|| {
                $code;
                Barrier::new(core_count)
            })
            .wait();
    }}

    // runs an initialization routine on each core
    // waits for this to complete before any core proceeds
    macro all($code:block) {{
        // needs to be in an extra block to avoid namespace collisions
        static BARRIER: Once<Barrier> = Once::new();
        $code;
        BARRIER.call_once(|| Barrier::new(core_count)).wait();
    }}

    // this is where the magic happens
    one!({ init_coroutine_queue() });
    all!({ set_up_idle() });
    all!({ init_coroutine_executor() });
    all!({ init_event_handler() });
    all!({ MP_STAGE.store(MPStage::MPPreempt, Ordering::SeqCst) });
    one!({
        kprintln!("Starting second round of device discovery...");
        discover_devices(false);
        kprintln!("Finished second round of device discovery.");
    });
    one!({
        spawn_thread(move || {
            kprintln!("Starting Testing Code...");
            Work::work();
        })
    });
    all!({ Arch::set_irq_enabled(true) });
    poll_tasks() // runs on all cores, never to return
}

// also copy-pasted from the tutorial
pub fn test_runner(tests: &'static [&(dyn Fn() + Send + Sync)]) {
    for test in tests {
        test();
    }
    Arch::shutdown(0);
}

#[cfg(test)]
#[panic_handler]
fn rust_panic(info: &core::panic::PanicInfo) -> ! {
    panic::rust_panic_impl(info);
}

pub macro integration_test($test:block) {
    use kernel_common::{KernelWorkTrait, system_init};

    #[cfg(test)]
    pub struct KernelWork;

    #[cfg(test)]
    impl KernelWorkTrait for KernelWork {
        fn work() {
            #[cfg(test)]
            $test;
            Arch::shutdown(0);
        }
    }

    #[cfg(test)]
    #[unsafe(no_mangle)]
    unsafe extern "C" fn system_main() -> ! {
        system_init::<KernelWork>();
    }

    #[panic_handler]
    fn rust_panic(info: &core::panic::PanicInfo) -> ! {
        kernel_common::panic::rust_panic_impl(info);
    }
}
