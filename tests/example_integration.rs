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
#![feature(custom_test_frameworks)]
#![test_runner(kernel_common::test_runner)]
#![reexport_test_harness_main = "test_main"]

use kernel_common::arch::{Arch, ArchTrait};
use kernel_common::KernelWorkTrait;
use kernel_common::print::kprintln;

#[cfg(test)]
pub struct KernelWork;

#[cfg(test)]
impl KernelWorkTrait for KernelWork {
    fn work() {
        #[cfg(test)]
        test_main();
        Arch::shutdown(0);
    }
}

#[cfg(test)]
#[unsafe(no_mangle)]
unsafe extern "C" fn system_main() -> ! {
    kernel_common::system_init::<KernelWork>();
}

#[panic_handler]
fn rust_panic(info: &core::panic::PanicInfo) -> ! {
    kernel_common::test_utils::rust_panic_test_impl(info);
}

#[test_case]
fn hello_world() {
    kprintln!("Hello world");
    Arch::shutdown(0);
}
