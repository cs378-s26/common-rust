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

use kernel_common::test_utils;
use kernel_common::system_init;
use kernel_common::KernelWork;

#[unsafe(no_mangle)]
unsafe extern "C" fn system_main() -> ! {
    system_init::<KernelWork>()
}

#[cfg(not(test))]
#[panic_handler]
fn rust_panic(info: &core::panic::PanicInfo) -> ! {
    test_utils::rust_panic_impl(info);
}