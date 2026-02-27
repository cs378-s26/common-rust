use core::{
    arch::naked_asm,
    mem::forget,
    ptr::{self},
};

use spin::MutexGuard;

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

impl const Default for Context {
    fn default() -> Self {
        Self {
            gp: GPRegisters { regs: [0; 31] },
            sp: Default::default(),
            pc: Default::default(),
            spsr: 0x5, // default to EL1h
        }
    }
}

fn slice_stack_pointer(slice: &[u8]) -> u64 {
    slice.as_ptr_range().end as u64
}

#[unsafe(naked)]
unsafe extern "C" fn jump_to_context(
    _buf: *const GPRegisters,
    _sp: u64,
    _spsr: u64,
    _pc: u64,
) -> ! {
    naked_asm!(
        // AAPCS64 call ABI:
        // x0 = buf, x1 = sp, x2 = spsr (unused here), x3 = pc
        "mov x16, x3",
        "mov sp, x1",
        // Restore callee-saved state + x0 argument register.
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
        "br x16",
        options(noreturn)
    )
}

impl Context {
    pub fn jump_to(&self) -> ! {
        unsafe { jump_to_context(&raw const self.gp, self.sp, self.spsr, self.pc) }
    }

    pub fn setup_kthread_context(&mut self) {
        // EL1h thread context value; currently only stored, not consumed by jump_to_context.
        self.spsr = 0x5;
    }

    pub fn setup_for_call<T>(
        &mut self,
        stack: &[u8],
        function: unsafe extern "C" fn(*mut T) -> !,
        data: *mut T,
    ) {
        self.setup_kthread_context();

        self.pc = function as usize as u64;
        self.gp.regs[0] = data as u64;
        self.sp = slice_stack_pointer(stack) & !0xF;
    }
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
            options(noreturn)
        )
    }

    unsafe { save_context_impl(slice_stack_pointer(stack), &raw mut ctx, &raw mut fwd) };

    // Ownership was moved through raw pointers into save_context_impl.
    forget(ctx);
    forget(fwd);
}
