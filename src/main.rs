#![no_std]
#![no_main]
#![feature(decl_macro)]

mod arch;
mod heap;
mod percore;
mod print;
mod sync;
mod thread;

use limine::BaseRevision;
use limine::request::{
    BootloaderInfoRequest, FirmwareTypeRequest, RequestsEndMarker, RequestsStartMarker,
    RsdpRequest, SmbiosRequest,
};
use talc::Span;

use crate::arch::{halt, initialize_mp};
use crate::heap::init_malloc;
use crate::print::init_tty;

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
static RSDP_REQUEST: RsdpRequest = RsdpRequest::new();

#[used]
#[unsafe(link_section = ".limine_requests")]
static SMBIOS_REQUEST: SmbiosRequest = SmbiosRequest::new();

#[used]
#[unsafe(link_section = ".limine_requests_start")]
static _START_MARKER: RequestsStartMarker = RequestsStartMarker::new();

#[used]
#[unsafe(link_section = ".limine_requests_end")]
static _END_MARKER: RequestsEndMarker = RequestsEndMarker::new();

static mut THE_HEAP: [u8; 6 * 1024 * 1024] = [0; _];

#[unsafe(no_mangle)]
unsafe extern "C" fn system_main() -> ! {
    assert!(BASE_REVISION.is_valid());
    init_tty();
    init_malloc(Span::from_slice(&raw mut THE_HEAP));

    // note we don't need to do anything special here because rust doesn't have init_array
    // if we wanted once-initialized data, we would either provide our custom mechanism,
    // or just spam OnceCell

    // handle SSE/FSGSBASE/etc in initialize_mp
    initialize_mp();
}

#[cfg(not(test))]
#[panic_handler]
fn rust_panic(info: &core::panic::PanicInfo) -> ! {
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
