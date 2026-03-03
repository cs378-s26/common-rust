// Dummy GIC/APIC-like interface for aarch64 build; any use will panic.
use core::sync::atomic::{AtomicU64};

pub fn send_ipi_all_except_self(_vector: u8) {
    panic!("send_ipi_all_except_self not implemented on aarch64");
}

pub fn eoi() {
    panic!("eoi not implemented on aarch64");
}

static TIMER_TICKS: AtomicU64 = AtomicU64::new(0);

pub fn timer_ticks() -> u64 {
    TIMER_TICKS.load(core::sync::atomic::Ordering::Relaxed)
}