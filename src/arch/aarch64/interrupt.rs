use crate::arch::{Arch, IrqStateTrait};

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
