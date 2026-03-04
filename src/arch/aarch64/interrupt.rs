use spin::Once;

use crate::{
    arch::{Arch, IrqStateTrait},
    print::{kprint, kprintln},
};

use crate::arch::aarch64::vmm;
use crate::device::device::FDT;
use crate::physical_memory::HHDM_REQUEST;
use crate::virtual_memory::PagingOptions;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

#[repr(transparent)]
#[derive(PartialEq, Eq, Clone, Copy)]
pub struct IrqState(bool);

// Minimal interrupt context placeholder; real implementation TBD.
#[repr(C)]
pub struct InterruptContext;

pub const IPI_WAKE_VECTOR: u8 = 0;

const DAIF_IRQ_BIT: u64 = 1 << 7;

pub unsafe fn disable() {
    unsafe { core::arch::asm!("msr daifset, #2", options(nomem, nostack, preserves_flags)) }
}

pub unsafe fn enable() {
    unsafe { core::arch::asm!("msr daifclr, #2", options(nomem, nostack, preserves_flags)) }
}

impl IrqStateTrait for IrqState {
    type Arch = Arch;
    #[inline(always)]
    fn save() -> IrqState {
        let daif: u64;
        unsafe {
            core::arch::asm!(
                "mrs {}, daif",
                lateout(reg) daif,
            )
        };
        IrqState((daif & DAIF_IRQ_BIT) == 0)
    }

    fn is_masked(&self) -> bool {
        !self.0
    }
}

/**
 * GIC & Timer
 */
pub const TIMER_HZ: u64 = 100;
pub static TIMER_INTERVAL: Once<u64> = Once::new();
pub static TIMER_TICKS: AtomicU64 = AtomicU64::new(0);
pub static GICC_BASE_VIRT: AtomicUsize = AtomicUsize::new(0);

const GICD_CTLR: usize = 0x000;
const GICD_ISENABLER0: usize = 0x100;
const GICD_IPRIORITYR: usize = 0x400;

const GICC_CTLR: usize = 0x000;
const GICC_PMR: usize = 0x004;
const GICC_BPR: usize = 0x008;
pub const GICC_IAR: usize = 0x00C; // read to acknowledge, returns INTID
pub const GICC_EOIR: usize = 0x010; // write INTID to signal end-of-interrupt

// gic v2
pub fn timer_frequency() -> u64 {
    let freq: u64;
    unsafe {
        core::arch::asm!("mrs {x}, cntfrq_el0", x = out(reg) freq);
    }
    freq
}

pub fn current_time_ns() -> u64 {
    let count: u64;
    unsafe { core::arch::asm!("mrs {}, cntpct_el0", out(reg) count) };
    count * 1_000_000_000 / timer_frequency()
}

pub unsafe fn timer_init() {
    unsafe {
        core::arch::asm!(
            "msr cntp_tval_el0, {x}",
            x = in(reg) *TIMER_INTERVAL.get().expect("Interval uninitialized"),
        );

        core::arch::asm!( // enables it
            "msr cntp_ctl_el0, {x}",
            x = in(reg) 1u64,  // ENABLE=1, IMASK=0
        );
    }
}

pub fn timer_ticks() -> u64 {
    TIMER_TICKS.load(Ordering::Relaxed)
}

fn gic_base_addrs() -> (usize, usize) {
    if let Some(dt) = FDT.get() {
        for node in dt.all_nodes() {
            let is_gic = node.properties().any(|p| {
                p.name == "compatible"
                    && p.as_str()
                        .map(|s| s.contains("arm,cortex-a15-gic"))
                        .unwrap_or(false)
            });
            if is_gic {
                if let Some(mut reg) = node.reg() {
                    let gicd = reg.next().unwrap().starting_address as usize;
                    let gicc = reg.next().unwrap().starting_address as usize;
                    kprintln!("GIC: GICD={:#x} GICC={:#x}", gicd, gicc);
                    return (gicd, gicc);
                }
            }
        }
    }
    kprintln!("GIC: FDT lookup failed, using hardcoded QEMU virt addresses");
    (0x0800_0000, 0x0801_0000)
}

pub fn gicd_init() {
    TIMER_INTERVAL.call_once(|| timer_frequency() / TIMER_HZ);

    let hhdm = HHDM_REQUEST.get_response().unwrap().offset() as usize;
    let (gicd_phys, gicc_phys) = gic_base_addrs();

    let gicd_virt = gicd_phys + hhdm;
    let gicc_virt = gicc_phys + hhdm;

    // map GICD
    let flags = PagingOptions::PRESENT | PagingOptions::WRITABLE;
    vmm::vmap(
        crate::arch::aarch64::vmm::get_address_space(),
        gicd_virt as u64,
        gicd_phys as u64,
        flags,
    );
    // map GICC
    vmm::vmap(
        crate::arch::aarch64::vmm::get_address_space(),
        gicc_virt as u64,
        gicc_phys as u64,
        flags,
    );

    GICC_BASE_VIRT.store(gicc_virt, Ordering::Release);

    unsafe {
        let gicd = gicd_virt as *mut u32;

        // disable gicd
        gicd.add(GICD_CTLR / 4).write_volatile(0);

        let pri_reg = (gicd_virt + GICD_IPRIORITYR + 28) as *mut u32; // offset 28 = intid 28..31
        let mut word = pri_reg.read_volatile();
        word &= !(0xFF << 16); // clear byte lane 2 (intid 30)
        word |= 0xA0 << 16; // set priority 0xA0
        pri_reg.write_volatile(word);

        // enable ppi 30
        let isenabler0 = gicd.add(GICD_ISENABLER0 / 4);
        isenabler0.write_volatile(1 << 30);

        // enable gicd
        gicd.add(GICD_CTLR / 4).write_volatile(1);
    }

    kprintln!("gicd_init done");
}

pub fn gicc_init() {
    let gicc_virt = GICC_BASE_VIRT.load(Ordering::Acquire);
    assert!(
        gicc_virt != 0,
        "gicc_init called before gicd_init mapped GICC"
    );

    unsafe {
        let gicc = gicc_virt as *mut u32;
        gicc.add(GICC_PMR / 4).write_volatile(0xF0); // take interrupts with priority < 0xF0

        gicc.add(GICC_BPR / 4).write_volatile(0);

        // enable cpu interface
        gicc.add(GICC_CTLR / 4).write_volatile(1);
    }

    // set ppi 30 for cores besides the bsp
    let hhdm = HHDM_REQUEST.get_response().unwrap().offset() as usize;
    let (gicd_phys, _) = gic_base_addrs();
    unsafe {
        let isenabler0 = (gicd_phys + hhdm + GICD_ISENABLER0) as *mut u32;
        isenabler0.write_volatile(1 << 30);
    }

    kprintln!("gicc_init done on core {}", crate::mp::CORE_ID.get());
}
