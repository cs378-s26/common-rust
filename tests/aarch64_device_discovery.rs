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

extern crate alloc;

use kernel_common::arch::{Arch, ArchTrait, KernelEntryTrait};
use kernel_common::print::kprintln;

pub struct TestEntry;

impl KernelEntryTrait for TestEntry {
    fn kernel_main() -> ! {
        // device discovery already ran inside system_init before we got here
        test_main();
        Arch::halt()
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn system_main() -> ! {
    kernel_common::system_init::<Arch, TestEntry>();
}

#[panic_handler]
fn rust_panic(info: &core::panic::PanicInfo) -> ! {
    kernel_common::test_utils::rust_panic_test_impl(info);
}

#[test_case]
fn device_discovery_ran() {
    // if we got here, system_init ran device discovery without panicking
    kprintln!("device discovery: ok");
}
