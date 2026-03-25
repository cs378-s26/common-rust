use core::sync::atomic::{AtomicU64, Ordering};

use spin::Once;

use crate::arch::aarch64::devices::a15_gic::{GICC_BASE_VIRT, GICD_BASE_VIRT};
use crate::print::kprintln;

pub const TIMER_HZ: u64 = 1;
pub static TIMER_INTERVAL: Once<u64> = Once::new();
pub static TIMER_TICKS: AtomicU64 = AtomicU64::new(0);

const GICD_ISENABLER0: usize = 0x100;

const GICC_CTLR: usize = 0x000;
const GICC_PMR: usize = 0x004;
const GICC_BPR: usize = 0x008;
const GICC_IAR: usize = 0x00C; // read to acknowledge, returns INTID
const GICC_EOIR: usize = 0x010; // write INTID to signal end-of-interrupt

// gic v2
fn timer_frequency() -> u64 {
    let freq: u64;
    unsafe {
        core::arch::asm!("mrs {x}, cntfrq_el0", x = out(reg) freq);
    }
    freq
}

pub fn timer_ticks() -> u64 {
    TIMER_TICKS.load(Ordering::Relaxed)
}

pub fn inc_timer_ticks() {
    TIMER_TICKS.fetch_add(1, Ordering::Relaxed);
}

pub fn timer_reset_interval() {
    unsafe {
        core::arch::asm!(
            "msr cntp_tval_el0, {x}",
            x = in(reg) *TIMER_INTERVAL.get().expect("Interval uninitialized"),
        );
    }
}

pub fn setup_timer() {
    TIMER_INTERVAL.call_once(|| timer_frequency() / TIMER_HZ);
    unsafe {
        timer_reset_interval();

        core::arch::asm!( // enables it
            "msr cntp_ctl_el0, {x}",
            x = in(reg) 1u64,  // ENABLE=1, IMASK=0
        );
    }
}

pub fn gicc_init() {
    let gicc_virt = GICC_BASE_VIRT.load(Ordering::Acquire);
    kprintln!("gicc_init: GICC_BASE_VIRT={:#x}", gicc_virt);

    assert!(
        gicc_virt != 0,
        "gicc_init called before gicd_init mapped GICC"
    );

    unsafe {
        let gicc = gicc_virt as *mut u32;
        // take interrupts with priority < 0xF0
        // lower the number, higher the priority | ie priority #1 is the most important
        gicc.add(GICC_PMR / 4).write_volatile(0xF0);

        // preemption group selection by binary point register
        // likely won't have to worry about this for a while
        // https://developer.arm.com/documentation/ihi0048/a/BEIHEBAG
        gicc.add(GICC_BPR / 4).write_volatile(0);

        // enable cpu interface
        gicc.add(GICC_CTLR / 4).write_volatile(1);
    }

    // accept interrupt 30 (generic timer) on all cores
    let gicd_virt = GICD_BASE_VIRT.load(Ordering::Acquire);
    unsafe {
        let isenabler0 = (gicd_virt + GICD_ISENABLER0) as *mut u32;
        isenabler0.write_volatile(1 << 30);
    }

    kprintln!("gicc_init done on core {}", crate::mp::CORE_ID.get());
}

// function appears pure but the act of reading signals the GIC hardware that the interrupt is being handled
pub fn ack_irq() -> u32 {
    let gicc_virt = GICC_BASE_VIRT.load(Ordering::Acquire);
    unsafe { (((gicc_virt + GICC_IAR) as *const u32).read_volatile()) & 0x3FF }
}

pub fn eoi(intid: u32) {
    let gicc_virt = GICC_BASE_VIRT.load(Ordering::Acquire);
    unsafe {
        ((gicc_virt + GICC_EOIR) as *mut u32).write_volatile(intid);
    }
}
