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
use core::{
    hint::spin_loop,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

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

use crate::{
    arch::{Arch, ArchTrait},
    cmdline::{get_cmdline_error, get_cmdline_text, parse_kernel_cmdline},
    coroutine::{init_coroutine_executor, init_coroutine_queue},
    devices::discovery::{create_drivers, discover_devices},
    event::init_event_handler,
    fs::{
        dev::{DEV, Dev},
        fake::{FAKE, Fake},
        vfs::VFS,
    },
    memory::{heap::init_malloc, virtual_memory_2::VirtualMemory},
    mp::{CORE_ID, MP_STAGE, MPStage, init_cpu_local_table},
    print::{StackTrace, init_tty, kprintln},
    process::init_pid_allocator,
    state::{Irq, StateTrait},
    thread::{poll_tasks, set_up_idle, spawn_thread, init_sleep},
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

#[cfg(target_arch = "x86_64")]
fn take_kb_input() {
    loop {
        let c = crate::devices::char::ps2_kb_m::read();
        kprintln!("Read character: {}", c);
    }
}

fn usual_main() {
    kprintln!("Entered kernel");
    #[cfg(target_arch = "x86_64")]
    take_kb_input();
}

pub struct KernelWork;

impl KernelWorkTrait for KernelWork {
    fn work() {
        #[cfg(test)]
        test_main();
        #[cfg(not(test))]
        usual_main();
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
    VFS.mount(fake, &["/", "fake"]).unwrap();
    let dev = Arc::clone(DEV.call_once(Dev::new));
    VFS.mount(dev, &["/", "dev"]).unwrap();

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

// This is really janky - the spin crates Once<Barrier> has issues on real hardware
// We replace this with a temporary implementation of a barrier for the sake of simplicity.
pub struct SpinBarrier {
    count: AtomicUsize,
    generation: AtomicUsize,
}

impl SpinBarrier {
    #[allow(clippy::new_without_default)]
    pub const fn new() -> Self {
        Self {
            count: AtomicUsize::new(0),
            generation: AtomicUsize::new(0),
        }
    }

    pub fn wait(&self, n_threads: usize) {
        let _gen = self.generation.load(Ordering::Acquire);

        let prev = self.count.fetch_add(1, Ordering::AcqRel);

        if prev + 1 == n_threads {
            self.count.store(0, Ordering::Release);
            self.generation.fetch_add(1, Ordering::AcqRel);
        } else {
            while self.generation.load(Ordering::Acquire) == _gen {
                spin_loop();
            }
        }
    }
}

/// wrapper around initalize core that goes to kernel main
/// # Safety
/// Should only be called from bootstrap processor during kernel initialization
unsafe extern "C" fn core_init<Work: KernelWorkTrait>(cpu: &Cpu) -> ! {
    unsafe { Arch::initialize_core(cpu) };
    //kprintln!("Done initializing core BSP");
    assert!(!Irq::get());
    let mp_res = MP_REQUEST
        .get_response()
        .expect("Expected to find MpResponse, found None.");
    let core_count = mp_res.cpus().len();
    //kprintln!("Core count: {}", core_count);
    //kprintln!("Meow 2");

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    kprintln!(
        "[{} {}/{}] processor init done",
        CORE_ID.get(),
        COUNTER.fetch_add(1, Ordering::Relaxed) + 1,
        core_count
    );

    // runs an initialization routine once overall
    // waits for this to complete before any core proceeds
    macro one($code:block) {{
        // needs to be in an extra block to avoid namespace collisions
        static BARRIER: SpinBarrier = SpinBarrier::new();
        static ONCE: AtomicBool = AtomicBool::new(false);

        if !ONCE.swap(true, Ordering::SeqCst) {
            $code;
        }

        BARRIER.wait(core_count);
    }}

    // runs an initialization routine on each core
    // waits for this to complete before any core proceeds
    macro all($code:block) {{
        // needs to be in an extra block to avoid namespace collisions
        static BARRIER: SpinBarrier = SpinBarrier::new();
        $code;
        BARRIER.wait(core_count);
    }}

    // this is where the magic happens
    one!({ init_coroutine_queue() });
    //kprintln!("Coroutines Initialized!");
    all!({ set_up_idle() });
    //kprintln!("Idle thread set up!");
    all!({ init_coroutine_executor() });
    all!({ init_event_handler() });
    one!({ init_sleep() });
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
    kprintln!("Interrupts enabled, polling tasks!");
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
