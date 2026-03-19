#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_common::test_runner)]
#![reexport_test_harness_main = "test_main"]

use kernel_common::KernelWorkTrait;
use kernel_common::print::kprintln;
use kernel_common::system_init;

#[cfg(test)]
pub struct KernelWork;

#[cfg(test)]
impl KernelWorkTrait for KernelWork {
    fn work() {
        #[cfg(test)]
        test_main();
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

#[test_case]
fn hello_world() {
    kprintln!("Hello world");
}
