use alloc::boxed::Box;
use crate::sync::MutexLike;
use crate::arch::{Arch, ArchTrait};
pub mod uart_pl011;

pub fn create_drivers() {
    // create drivers for devices that are specific to this architecture, for example aarch64's uart_pl011
    let mut drivers = crate::devices::device_discovery::SYSTEM_DRIVERS.lock();
    drivers.push(Box::new(uart_pl011::UartPl011Discovery));
    Arch::create_arch_specific_drivers(&mut drivers);
}