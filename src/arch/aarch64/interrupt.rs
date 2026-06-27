use super::context::GPRegisters;

// Minimal interrupt context placeholder; real implementation TBD.
#[repr(C)]
pub struct InterruptContext {
    pub gpr: GPRegisters,
    pub sp: u64,
    pub pc: u64,
    pub spsr: u64,
    pub tpidr_el0: u64,
}

pub const IPI_WAKE_VECTOR: u8 = 0;

const DAIF_IRQ_BIT: u64 = 1 << 7;

pub fn disable() {
    unsafe { core::arch::asm!("msr daifset, #2", options(nomem, nostack, preserves_flags)) }
}

pub fn enable() {
    unsafe { core::arch::asm!("msr daifclr, #2", options(nomem, nostack, preserves_flags)) }
}

pub fn irq_is_enabled() -> bool {
    let daif: u64;
    unsafe {
        core::arch::asm!("mrs {}, daif", out(reg) daif);
    }
    // I bit is bit 7: 1 means masked, 0 means enabled
    (daif & DAIF_IRQ_BIT) == 0
}
