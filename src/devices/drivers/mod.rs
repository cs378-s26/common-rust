use crate::arch::{Arch, ArchTrait};
use crate::sync::MutexLike;
use alloc::boxed::Box;
use crate::devices::char::uart_pl011::UartPl011Discovery;
use crate::devices::device_discovery::SYSTEM_DRIVERS;

pub fn create_drivers() {
    // create drivers for devices that are specific to this architecture, for example aarch64's uart_pl011
    let mut drivers = SYSTEM_DRIVERS.lock();
    drivers.push(Box::new(UartPl011Discovery));
    Arch::create_arch_specific_drivers(&mut drivers);
}
