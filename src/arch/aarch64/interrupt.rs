use crate::{
    arch::{Arch, IrqStateTrait},
    print::kprintln,
};
use core::arch::{asm, global_asm};

#[repr(transparent)]
#[derive(PartialEq, Eq, Clone, Copy)]
pub struct IrqState(bool);

global_asm!(include_str!("exception.S"), options(raw));

unsafe extern "C" {
    static __exception_vector_start: u8;
}
#[repr(C)]
struct ExceptionContext {
    /// General Purpose Registers.
    gpr: [u64; 30],
    /// The link register, aka x30.
    lr: u64,
    /// Exception link register. The program counter at the time the exception happened.
    elr_el1: u64,
    /// Saved program status.
    spsr_el1: u64,
    /// Exception syndrome register.
    esr_el1: u64,
}

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

pub fn init_exceptions() {
    kprintln!("Enabling interrupts");
    return;
    let mut sp: u64;
    _ = sp;

    // limine starts us on a user level stack (SP_EL0), so we switch to the kernel stack (SP_EL1) using SPSel.
    // this also ensures the right exception path will be called.
    unsafe {
        core::arch::asm!(
            "mov x0, sp",
            "msr SPSel, #1",
            "mov sp, x0",
            options(nomem, nostack, preserves_flags),
        );
    }
    // TODO handle setting up kernel stack for coming back from lower EL (using SPSel)
    let table_start = core::ptr::addr_of!(__exception_vector_start) as usize;

    unsafe {
        asm!(
            "msr vbar_el1, {}",
            "isb",
            in(reg) table_start,
            options(nomem, nostack, preserves_flags),
        );
    }
}

#[unsafe(no_mangle)]
extern "C" fn current_el0(context: &mut ExceptionContext) {
    _ = context;
    panic!("current_el0 exception handler not implemented");
}

#[unsafe(no_mangle)]
extern "C" fn lower_aarch64(context: &mut ExceptionContext) {
    _ = context;
    panic!("lower_aarch64 exception handler not implemented");
}

#[unsafe(no_mangle)]
extern "C" fn current_elx(context: &mut ExceptionContext) {
    _ = context;
    panic!("current_elx exception handler not implemented");
}
