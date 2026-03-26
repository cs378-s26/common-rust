use crate::arch::{Arch, ArchTrait};
use crate::sync::MutexLike;
use alloc::boxed::Box;
pub mod uart_pl011;
pub mod virtio_discovery;

pub fn create_drivers() {
    // create drivers for devices that are specific to this architecture, for example aarch64's uart_pl011
    let mut drivers = crate::devices::device_discovery::SYSTEM_DRIVERS.lock();
    drivers.push(Box::new(uart_pl011::UartPl011Discovery));
    drivers.push(Box::new(virtio_discovery::VirtioDiscovery));
    Arch::create_arch_specific_drivers(&mut drivers);
}
