#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_common::test_runner)]

kernel_common::integration_test!({
    use kernel_common::{devices::discovery::acpi::ps2_supported, print::kprintln};

    kprintln!("Testing PS2 disabled...");
    let ps2 = ps2_supported();
    kprintln!("PS2 supported: {:?}", ps2);
    kprintln!("Done.");
});
