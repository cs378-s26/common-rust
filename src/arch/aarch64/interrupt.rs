use crate::{
    arch::{Arch, IrqStateTrait},
    print::{kprint, kprintln},
};

#[repr(transparent)]
#[derive(PartialEq, Eq, Clone, Copy)]
pub struct IrqState(bool);

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

pub unsafe fn timer_init(interval_ticks: u64) {
    unsafe {
        core::arch::asm!(
            "msr cntp_tval_el0, {x}",
            x = in(reg) interval_ticks,
        );

        core::arch::asm!(
            "msr cntp_ctl_el0, {x}",
            x = in(reg) 1u64,  // ENABLE=1, IMASK=0
        );
    }
}

pub fn timer_frequency() -> u64 {
    let freq: u64;
    unsafe {
        core::arch::asm!("mrs {x}, cntfrq_el0", x = out(reg) freq);
    }
    kprintln!("Frequency is {}", freq);
    freq
}
