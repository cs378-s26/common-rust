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

use kernel_common::KernelWork;
#[cfg(not(test))]
use kernel_common::panic;
use kernel_common::system_init;

#[unsafe(no_mangle)]
unsafe extern "C" fn system_main() -> ! {
    system_init::<KernelWork>()
}

#[cfg(not(test))]
#[panic_handler]
fn rust_panic(info: &core::panic::PanicInfo) -> ! {
    panic::rust_panic_impl(info);
}
