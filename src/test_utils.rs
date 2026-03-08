use crate::arch::{Arch, ArchTrait};
use crate::print::kprintln;
use crate::*;

#[inline(always)]
#[allow(dead_code)]
pub fn rust_panic_test_impl(info: &core::panic::PanicInfo) -> ! {
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
