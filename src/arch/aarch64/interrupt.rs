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
