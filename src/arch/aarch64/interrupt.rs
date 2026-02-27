#[repr(transparent)]
#[derive(PartialEq, Eq, Clone, Copy)]
pub struct IrqState(bool);

const DAIF_IRQ_BIT: u64 = 1 << 7;

unsafe fn disable() {
    unsafe { core::arch::asm!("msr daifset, #2", options(nomem, nostack, preserves_flags)) }
}

unsafe fn enable() {
    unsafe { core::arch::asm!("msr daifclr, #2", options(nomem, nostack, preserves_flags)) }
}

impl IrqState {
    #[inline(always)]
    pub fn save() -> IrqState {
        let daif: u64;
        unsafe {
            core::arch::asm!(
                "mrs {}, daif",
                lateout(reg) daif,
            )
        };
        IrqState((daif & DAIF_IRQ_BIT) == 0)
    }

    #[inline(always)]
    pub fn restore(self) {
        if self.0 {
            unsafe { enable() };
        } else {
            unsafe { disable() };
        }
    }

    pub fn is_masked(self) -> bool {
        !self.0
    }
}

#[inline(always)]
pub fn irq_disable() {
    unsafe { disable() };
}

#[inline(always)]
pub fn irq_enable() {
    unsafe { enable() };
}
