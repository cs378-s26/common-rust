// Dummy GIC/APIC-like interface for aarch64 build.
use core::sync::atomic::AtomicU64;

// TODO: wire up real GIC SGIs; stubbed to avoid panics in tests.
pub fn send_ipi_all_except_self(_vector: u8) {}

pub fn eoi() {}

static TIMER_TICKS: AtomicU64 = AtomicU64::new(0);

pub fn timer_ticks() -> u64 {
    TIMER_TICKS.load(core::sync::atomic::Ordering::Relaxed)
}
