use crate::{
    arch::{Arch, ArchTrait},
    print::kprintln,
    *,
};

#[inline(always)]
#[allow(dead_code)]
pub fn rust_panic_impl(info: &core::panic::PanicInfo) -> ! {
    match info.location() {
        Some(location) => kprintln!(
            "panic: {}\n{}:{}:{}\n{}",
            info.message(),
            location.file(),
            location.line(),
            location.column(),
            StackTrace::current()
        ),
        None => kprintln!(
            "panic: {}\nunknown location\n{}",
            info.message(),
            StackTrace::current()
        ),
    };

    Arch::shutdown(10_u16);
    Arch::halt()
}
