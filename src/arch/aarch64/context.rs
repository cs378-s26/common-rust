use core::{
    arch::{asm, naked_asm},
    mem::forget,
    ptr::{self},
};

use spin::MutexGuard;

use super::interrupt::InterruptContext;
use crate::arch::{Arch, ContextTrait};

const SPSR_MODE_EL1H: u64 = 0x5;
const SPSR_DAIF_MASK: u64 = 0b1111 << 6;
const SPSR_IRQ_MASK: u64 = 1 << 7;

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct GPRegisters {
    pub regs: [u64; 31],
}

#[derive(Debug, Clone, Copy)]
pub struct Context {
    pub gp: GPRegisters,
    pub sp: u64,
    pub pc: u64,
    pub spsr: u64,
}

fn slice_stack_ptr(stack: &[u8]) -> u64 {
    assert!(stack.as_ptr_range().end as u64 & 0xF == 0);
    stack.as_ptr_range().end as u64
}

impl ContextTrait for Context {
    type Arch = Arch;
    fn setup_kthread_context(&mut self) {
        self.spsr = SPSR_MODE_EL1H;
    }

    fn jump_to(&self) -> ! {
        unsafe {
            jump_to_context(
                &raw const self.gp,
                self.sp,
                self.spsr,
                self.pc,
                ((self.spsr & 0b1111) == 0) as u64,
            );
        }
    }

    fn new_kthread<T>(
        stack: &[u8],
        function: unsafe extern "C" fn(*mut T) -> !,
        data: *mut T,
    ) -> Self {
        let mut ctx = Self::default();
        ctx.setup_kthread_context();

        ctx.pc = function as usize as u64;
        ctx.gp.regs[0] = data as u64;
        ctx.sp = slice_stack_ptr(stack) & !0xF;
        ctx
    }

    fn new_uthread(pc: *const u8, sp: *const u8) -> Self {
        let mut ctx = Self::default();
        ctx.spsr = 0;
        ctx.pc = pc as u64;
        ctx.sp = sp as u64;
        ctx
    }
}

impl const Default for Context {
    fn default() -> Self {
        Self {
            gp: GPRegisters { regs: [0; 31] },
            sp: Default::default(),
            pc: Default::default(),
            spsr: SPSR_MODE_EL1H | SPSR_IRQ_MASK, // EL1h with IRQ masked
        }
    }
}

impl Context {
    pub fn save_from_interrupt(&mut self, ctx: &InterruptContext) {
        self.gp = ctx.gpr;
        self.sp = ctx.sp;
        self.pc = ctx.pc;
        self.spsr = ctx.spsr;
    }
}

#[unsafe(naked)]
unsafe extern "C" fn jump_to_context(
    _buf: *const GPRegisters,
    _sp: u64,
    _spsr: u64,
    _pc: u64,
    _el: u64,
) -> ! {
    naked_asm!(
        // AAPCS64 call ABI:
        // x0 = buf, x1 = sp, x2 = spsr, x3 = pc, x4 = is_user
        "cmp x4, #1",
        "b.eq 1f",
        "mov sp, x1",
        "b 2f",
        "1:",
        "msr sp_el0, x1",
        "2:",
        // set things up for eret
        "msr spsr_el1, x2",
        "msr elr_el1, x3",
        // Restore callee-saved state + x0 argument register.
        "ldr x1, [x0, #8]",
        "ldr x2, [x0, #16]",
        "ldr x3, [x0, #24]",
        "ldr x4, [x0, #32]",
        "ldr x5, [x0, #40]",
        "ldr x6, [x0, #48]",
        "ldr x7, [x0, #56]",
        "ldr x8, [x0, #64]",
        "ldr x9, [x0, #72]",
        "ldr x10, [x0, #80]",
        "ldr x11, [x0, #88]",
        "ldr x12, [x0, #96]",
        "ldr x13, [x0, #104]",
        "ldr x14, [x0, #112]",
        "ldr x15, [x0, #120]",
        "ldr x16, [x0, #128]",
        "ldr x17, [x0, #136]",
        "ldr x18, [x0, #144]",
        "ldr x19, [x0, #152]",
        "ldr x20, [x0, #160]",
        "ldr x21, [x0, #168]",
        "ldr x22, [x0, #176]",
        "ldr x23, [x0, #184]",
        "ldr x24, [x0, #192]",
        "ldr x25, [x0, #200]",
        "ldr x26, [x0, #208]",
        "ldr x27, [x0, #216]",
        "ldr x28, [x0, #224]",
        "ldr x29, [x0, #232]",
        "ldr x30, [x0, #240]",
        "ldr x0, [x0, #0]",
        "eret"
    )
}

#[repr(C)]
struct StackContextFrame {
    x19: u64,
    x20: u64,
    x21: u64,
    x22: u64,
    x23: u64,
    x24: u64,
    x25: u64,
    x26: u64,
    x27: u64,
    x28: u64,
    x29: u64,
    x30: u64,
    sp: u64,
}

#[allow(unused_assignments)]
pub unsafe fn save_context<T: FnOnce() -> !>(
    stack: &[u8],
    mut ctx: MutexGuard<'static, Context>,
    mut fwd: T,
) {
    unsafe extern "C" fn save_context_save<T: FnOnce() -> !>(
        frame: *const StackContextFrame,
        ctx: *mut MutexGuard<'static, Context>,
        fwd: *mut T,
    ) -> ! {
        let frame = unsafe { &*frame };
        let fwd: T = unsafe { ptr::read(fwd) };
        let mut ctx: MutexGuard<'static, Context> = unsafe { ptr::read(ctx) };

        ctx.gp.regs[19] = frame.x19;
        ctx.gp.regs[20] = frame.x20;
        ctx.gp.regs[21] = frame.x21;
        ctx.gp.regs[22] = frame.x22;
        ctx.gp.regs[23] = frame.x23;
        ctx.gp.regs[24] = frame.x24;
        ctx.gp.regs[25] = frame.x25;
        ctx.gp.regs[26] = frame.x26;
        ctx.gp.regs[27] = frame.x27;
        ctx.gp.regs[28] = frame.x28;
        ctx.gp.regs[29] = frame.x29;
        ctx.gp.regs[30] = frame.x30;
        ctx.sp = frame.sp;
        ctx.pc = frame.x30;
        let daif: u64;
        unsafe {
            asm!(
                "mrs {}, daif",
                lateout(reg) daif,
                options(nomem, nostack, preserves_flags),
            );
        }
        ctx.spsr = SPSR_MODE_EL1H | (daif & SPSR_DAIF_MASK);

        drop(ctx);
        fwd();
    }

    #[unsafe(naked)]
    unsafe extern "C" fn save_context_impl<T: FnOnce() -> !>(
        _stack: u64,
        _ctx: *mut MutexGuard<'static, Context>,
        _fwd: *mut T,
    ) {
        naked_asm!(
            // x0 = temporary stack top, x1 = ctx, x2 = fwd
            "mov x9, sp",
            "mov sp, x0",

            // frame: x19..x30 + original sp (13 * 8 = 104 bytes)
            "sub sp, sp, #112",
            "stp x19, x20, [sp, #0]",
            "stp x21, x22, [sp, #16]",
            "stp x23, x24, [sp, #32]",
            "stp x25, x26, [sp, #48]",
            "stp x27, x28, [sp, #64]",
            "stp x29, x30, [sp, #80]",
            "str x9, [sp, #96]",

            "mov x0, sp",
            "bl {save}",
            "brk #0",
            save = sym save_context_save::<T>,
        )
    }

    unsafe { save_context_impl(slice_stack_ptr(stack), &raw mut ctx, &raw mut fwd) };

    // Ownership was moved through raw pointers into save_context_impl.
    forget(ctx);
    forget(fwd);
}
