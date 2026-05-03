use core::arch::asm;

use crate::{
    arch::{Arch, ArchTrait},
    print::kprintln,
    *,
};

#[inline(always)]
pub fn assert_aligned() {
    let rsp: u64;
    unsafe {
        asm!("mov {}, rsp", out(reg) rsp);
    }
    let rip: u64;
    unsafe {
        asm!("lea {}, [rip]", out(reg) rip);
    }
    kprintln!("rsp: {:x}, rip: {:x}", rsp, rip);
    assert!(rsp.is_multiple_of(16), "misaligned stack pointer: {}", rsp);
}

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
