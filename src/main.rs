#![no_std]
#![no_main]
#![feature(decl_macro)]

mod arch;
mod sync;
mod print;

use limine::BaseRevision;
use limine::request::{
    BootloaderInfoRequest, FirmwareTypeRequest, RequestsEndMarker, RequestsStartMarker,
    RsdpRequest, SmbiosRequest,
};

use crate::arch::halt;
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

#[unsafe(no_mangle)]
unsafe extern "C" fn system_main() -> ! {
    assert!(BASE_REVISION.is_valid());
    init_tty();
    halt();
}

#[cfg(not(test))]
#[panic_handler]
fn rust_panic(info: &core::panic::PanicInfo) -> ! {
    halt()
}
