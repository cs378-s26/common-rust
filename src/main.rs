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

use core::arch::{aarch64, asm};
use core::sync::atomic::{AtomicU64, Ordering};

// For coroutines.
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use kernel_common::arch::{self, Arch, ArchTrait, KernelEntryTrait};
use kernel_common::cmdline::{get_cmdline_error, get_cmdline_text, parse_kernel_cmdline};
use kernel_common::coroutine::{init_coroutine_executor, init_coroutine_queue};
use kernel_common::device::device::{init_device_tree, map_virtio_devices};
use kernel_common::heap::init_malloc;
use kernel_common::mp::{CORE_ID, MP_STAGE, MPStage, init_cpu_local_table};
use kernel_common::physical_memory::{self, THE_HEAP, init_physical_memory_allocator};
use kernel_common::print::{init_tty, kprintln};
use kernel_common::thread::{init_threading, poll_tasks, set_up_idle, spawn_thread};
use kernel_common::virtual_memory::init_virtual_memory_allocator;
use limine::BaseRevision;
use limine::firmware_type::FirmwareType;
use limine::request::{
    BootloaderInfoRequest, FirmwareTypeRequest, MpRequest, RequestsEndMarker, RequestsStartMarker,
};
use spin::{Barrier, Once};
use talc::Span;

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

fn dump_boot_info() {
    if let Some(res) = BOOTLOADER_INFO_REQUEST.get_response() {
        kprintln!("bootloader: {} v{}", res.name(), res.version());
    }

    if let Some(res) = get_cmdline_text() {
        kprintln!("cmdline: \"{}\"", res);
    }

    if let Some(err) = get_cmdline_error() {
        match err {
            kernel_common::cmdline::CmdlineError::NoResponse => {
                kprintln!("warn: no response received for cmdline request")
            }
            kernel_common::cmdline::CmdlineError::Utf8Error(err) => {
                kprintln!("warn: failed to convert cmdline to utf8: {}", err)
            }
            kernel_common::cmdline::CmdlineError::ParseError(err) => {
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

struct MainKernelEntry;

impl KernelEntryTrait for MainKernelEntry {
    fn kernel_main() -> ! {
        kernel_main()
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn system_main() -> ! {
    assert!(BASE_REVISION.is_valid());

    parse_kernel_cmdline();
    init_tty();

    // print some system info
    if let Some(rev) = BASE_REVISION.loaded_revision() {
        // kprintln!("limine rev: {}", rev);
    }

    if let Some(res) = BOOTLOADER_INFO_REQUEST.get_response() {
        // kprintln!("bootloader: {} v{}", res.name(), res.version());
    }

    if let Some(res) = FIRMWARE_TYPE_REQUEST.get_response() {
        // kprintln!(
        //     "fimrware: {}",
        //     match res.firmware_type() {
        //         FirmwareType::X86_BIOS => "bios",
        //         FirmwareType::UEFI_32 => "uefi (32-bit)",
        //         FirmwareType::UEFI_64 => "uefi (64-bit)",
        //         FirmwareType::SBI => "sbi",
        //         _ => "unknown",
        //     }
        // )
    }

    if let Some(err) = get_cmdline_error() {
        match err {
            kernel_common::cmdline::CmdlineError::NoResponse => {
                kprintln!("warn: no response received for cmdline request")
            }
            kernel_common::cmdline::CmdlineError::Utf8Error(err) => {
                kprintln!("warn: failed to convert cmdline to utf8: {}", err)
            }
            kernel_common::cmdline::CmdlineError::ParseError(err) => {
                kprintln!("warn: failed to parse cmdline: {}", err)
            }
        }
    }

    if let Some(res) = get_cmdline_text() {
        kprintln!("cmdline: \"{}\"", res);
    }

    init_malloc(Span::from_slice(&raw mut THE_HEAP));

    init_physical_memory_allocator();
    init_virtual_memory_allocator();

    init_device_tree();

    // map_virtio_devices();

    // note we don't need to do anything special here because rust doesn't have init_array
    // if we wanted once-initialized data, we would either provide our custom mechanism,
    // or just spam OnceCell

    // handle SSE/FSGSBASE/etc in initialize_mp
    kprintln!("A");
    let mp_res = MP_REQUEST
        .get_response()
        .expect("Expected to find MpResponse, found None.");

    init_cpu_local_table(mp_res.cpus().len());
    kprintln!("B");

    Arch::initialize_mp::<MainKernelEntry>(&MP_REQUEST)
}

static INIT_THREADING_BARRIER: Once<Barrier> = Once::new();
static MP_PREEMPT_ENTER_BARRIER: Once<Barrier> = Once::new();
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn kernel_main() -> ! {
    // kprintln!("we are the MPCorelings! please feed us!");
    let mp_res = MP_REQUEST
        .get_response()
        .expect("Expected to find MpResponse, found None.");
    let core_count = mp_res.cpus().len();

    INIT_THREADING_BARRIER
        .call_once(|| {
            kprintln!("preparing common tasks on {}", CORE_ID.get());
            kprintln!("there are {} cores total", core_count);
            init_threading();
            init_coroutine_queue();
            Barrier::new(core_count)
        })
        .wait();

    let idle = set_up_idle();

    // kprintln!("init tid: core={}, {}", CORE_ID.get(), idle.tid());

    init_coroutine_executor();
    // kprintln!("Coroutine executor initialized.");

    MP_PREEMPT_ENTER_BARRIER
        .call_once(|| Barrier::new(core_count))
        .wait();

    MP_STAGE.store(MPStage::MPPreempt, Ordering::SeqCst);

    let initial_core = CORE_ID.get();

    // let GICD_BASE: usize = 0x0800_0000 + HHDM_REQUEST.get_response().unwrap().offset() as usize;
    // let gicd_iidr = (GICD_BASE + 0xFEC) as *const u32;
    // let iidr = unsafe { gicd_iidr.read_volatile() };
    // let gic_version = (iidr >> 4) & 0xF; // 0x2 = GICv2, 0x3 = GICv3
    // kprintln!("GIC VERSION: {}", gic_version);

    for i in 0..4 {
        spawn_thread(move || {
            kprintln!("Thread Start, id={}, initial_core={}", i, initial_core);

            // // dump_core_state("before");
            // unsafe {
            //     core::arch::asm!("svc {imm}", imm = const 8);
            // }
            // dump_core_state("after");

            // unsafe { asm!("mov x1, #8", "ldr x0, [x1]") }
            kprintln!(
                "Thread Ending from {}, id={}, initial_core={}, tid={}, fetch={}",
                CORE_ID.get(),
                i,
                initial_core,
                Thread::this_tid(),
                TEMP_COUNTER.fetch_add(1, Ordering::Relaxed) + 1
            );

            loop {
                yield_thread();
            }
        });
    }

    Arch::set_irq_enabled(true);
    poll_tasks()
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
