use core::{
    arch::naked_asm,
    mem::forget,
    ptr::{self},
};

use super::interrupt::InterruptContext;

use spin::MutexGuard;
use x86::{Ring, bits64::rflags::RFlags, segmentation::SegmentSelector};

use crate::arch::x86_64::{slice_stack_pointer, tables::GlobalDescriptorTable};
use crate::arch::{Arch, ContextTrait};

/// Represents the general-purpose registers of an x86 CPU.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct GPRegisters {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub rsp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct Context {
    pub gp: GPRegisters,
    pub rip: u64,
    pub rflags: RFlags,
    pub cs: u64,
    pub ss: u64,
}

impl const Default for Context {
    fn default() -> Self {
        Self {
            gp: GPRegisters {
                rax: 0,
                rbx: 0,
                rcx: 0,
                rdx: 0,
                rsi: 0,
                rdi: 0,
                rbp: 0,
                rsp: 0,
                r8: 0,
                r9: 0,
                r10: 0,
                r11: 0,
                r12: 0,
                r13: 0,
                r14: 0,
                r15: 0,
            },
            rip: Default::default(),
            rflags: RFlags::empty(),
            cs: Default::default(),
            ss: Default::default(),
        }
    }
}

#[unsafe(naked)]
unsafe extern "C" fn jump_to_context(
    // %rdi
    buf: *const GPRegisters,
    // %rsi
    ss: u64,
    // %rdx
    rsp: u64,
    // %rcx
    rflags: u64,
    // %r8
    cs: u64,
    // %r9
    rip: u64,
) -> ! {
    // TODO: we need to switch segmentation registers here, as well as fs and gs (for userspace)
    naked_asm!(
        // Prepare the stack frame for iretq
        // We use iretq here to switch, because it is possible the target context is userspace
        // See:
        // - Intel SDM Volume 2, Chapter 3, Section 1.3, IRET/IRETD/IRETQ—Interrupt Return
        // - INtel SDM Volume 3a, Chapter 7, Section 7.14.3 "IRET in IA-32e Mode"
        //
        // In essence, iretq will pop the following from the stack
        // SS
        // RSP
        // RFLAGS
        // CS
        // RIP <- current %rsp
        //
        // Basically this is a fused stack-switch + long jump.
        "pushq %rsi",
        "pushq %rdx",
        "pushq %rcx",
        "pushq %r8",
        "pushq %r9",
        // Restore registers
        "movq (0 * 8)(%rdi), %rax",
        "movq (1 * 8)(%rdi), %rbx",
        "movq (2 * 8)(%rdi), %rcx",
        "movq (3 * 8)(%rdi), %rdx",
        "movq (4 * 8)(%rdi), %rsi",
        // "movq (5 * 8)(%rdi), %rdi" - avoid restoring %rdi here, because it still holds the
        // context struct pointer.
        "movq (6 * 8)(%rdi), %rbp",
        // "movq (7 * 8)(%rdi), %rsp" - avoid loading SP, comment here to help the reader remember
        // that we haven't forgot about %rsp
        "movq (8 * 8)(%rdi), %r8",
        "movq (9 * 8)(%rdi), %r9",
        "movq (10 * 8)(%rdi), %r10",
        "movq (11 * 8)(%rdi), %r11",
        "movq (12 * 8)(%rdi), %r12",
        "movq (13 * 8)(%rdi), %r13",
        "movq (14 * 8)(%rdi), %r14",
        "movq (15 * 8)(%rdi), %r15",
        // %rdi needs to be the last thing restored, because we don't want to clobber our context
        // strict pointer
        "movq (5 * 8)(%rdi), %rdi",
        // Go to the context
        "iretq",
        options(att_syntax)
    )
}

impl ContextTrait for Context {
    type Arch = Arch;

    fn jump_to(&self) -> ! {
        unsafe {
            jump_to_context(
                &raw const self.gp,
                self.ss,
                self.gp.rsp,
                self.rflags.bits(),
                self.cs,
                self.rip,
            )
        }
    }

    fn setup_kthread_context(&mut self) {
        self.cs = SegmentSelector::new(GlobalDescriptorTable::CS, Ring::Ring0)
            .bits()
            .into();

        self.ss = SegmentSelector::new(GlobalDescriptorTable::DS, Ring::Ring0)
            .bits()
            .into();

        // make sure IF is set
        self.rflags.insert(RFlags::FLAGS_IF);
    }

    fn new_kthread<T>(
        stack: &[u8],
        function: unsafe extern "C" fn(*mut T) -> !,
        data: *mut T,
    ) -> Self {
        let mut ctx = Self::default();
        ctx.setup_kthread_context();

        ctx.rip = function as usize as u64;
        ctx.gp.rdi = data as u64;
        ctx.gp.rsp = slice_stack_pointer(stack);
        ctx
    }
}

impl Context {
    /// Populates this context from an interrupt frame, for preemptive context save.
    /// cs/ss are intentionally left untouched (already correct from setup_kthread_context).
    pub fn save_from_interrupt(&mut self, ctx: &InterruptContext) {
        // regs[] layout (low to high in memory = last-pushed to first-pushed):
        // [0]=r15 [1]=r14 [2]=r13 [3]=r12 [4]=r11 [5]=r10 [6]=r9 [7]=r8
        // [8]=rdi [9]=rsi [10]=rbx [11]=rdx [12]=rcx [13]=rax
        self.gp.r15 = ctx.regs[0];
        self.gp.r14 = ctx.regs[1];
        self.gp.r13 = ctx.regs[2];
        self.gp.r12 = ctx.regs[3];
        self.gp.r11 = ctx.regs[4];
        self.gp.r10 = ctx.regs[5];
        self.gp.r9 = ctx.regs[6];
        self.gp.r8 = ctx.regs[7];
        self.gp.rdi = ctx.regs[8];
        self.gp.rsi = ctx.regs[9];
        self.gp.rbx = ctx.regs[10];
        self.gp.rdx = ctx.regs[11];
        self.gp.rcx = ctx.regs[12];
        self.gp.rax = ctx.regs[13];
        self.gp.rbp = ctx.rbp;
        self.gp.rsp = ctx.rsp;
        self.rip = ctx.rip;
        self.rflags = RFlags::from_bits_truncate(ctx.rflags);
    }
}

#[repr(C)]
struct StackContextFrame {
    rip: u64,
    rflags: u64,
    r15: u64,
    r14: u64,
    r13: u64,
    r12: u64,
    rbp: u64,
    rsp: u64,
    rbx: u64,
}

/// This function saves the current context (CPU registers, stack pointer, etc.) of a thread
/// (or CPU state) onto a separate stack. After saving the context, the provided callback
/// function `fwd` is invoked. The callback will be executed in a "limbo" state, where no active
/// thread is running, interrupts are disabled, and no preemption occurs. This should only be
/// called by thread-internal logic.
///
/// # Safety
/// The callback function (`fwd`) is executed after the context is saved. The consumer of this
/// function must ensure that the callback does not assume that any thread-specific data is still
/// valid once the context is saved, as this function operates in a limbo state where no active thread
/// is running. The callback should also avoid performing any operations that could block, schedule, or
/// modify thread-local state, as this could lead to undefined behavior or deadlock. It is almost
/// impossible to use safely outside of the very specific callsite in `thread.rs`, because some
/// extra (undocumented) bookkeeping is required.
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
        // Read and move these arguments into local - these are on the temp stack now.
        let fwd: T = unsafe { ptr::read(fwd) };
        let mut ctx: MutexGuard<'static, Context> = unsafe { ptr::read(ctx) };

        ctx.gp.rbx = frame.rbx;
        ctx.gp.rsp = frame.rsp;
        ctx.gp.rbp = frame.rbp;
        ctx.gp.r12 = frame.r12;
        ctx.gp.r13 = frame.r13;
        ctx.gp.r14 = frame.r14;
        ctx.gp.r15 = frame.r15;
        ctx.rip = frame.rip;
        ctx.rflags = RFlags::from_bits(frame.rflags).unwrap();

        // At this point, it is safe to resume the current thread on any other core.
        // All relevant context variables are moved onto the new temporary stack.
        drop(ctx);

        fwd();
    }

    #[unsafe(naked)]
    unsafe extern "C" fn save_context_impl<T: FnOnce() -> !>(
        // %rdi
        stack: u64,
        // %rsi
        ctx: *mut MutexGuard<'static, Context>,
        // %rdx
        fwd: *mut T,
    ) {
        // SysV ABI: "Functions preserve the registers rbx, rsp, rbp, r12, r13, r14, and r15"
        // We should store them in the context, and jump to the rust handler implementation.
        //
        // Strictly speaking, this is probably not ABI compliant.
        naked_asm!(
            // Fake a frame - used for restoring some stuff.
            "pushq %rbp",
            "movq %rsp, %rbp",

            // Here, we have to switch stacks and save the callee-saved registers. We need to do
            // this without the help of x86's interrupt handler facilities (which automatically
            // switch stack for us on interrupt), since this is not an interrupt handler.

            // Use r11 as a scratch register to hold rsp
            "movq %rsp, %r11",

            // Set rsp = stack, switch off the main call stack
            "movq %rdi, %rsp",

            // Push things onto the *new* stack
            // Technically, we could just push this on the kernel's stack, but then we would lose
            // observability when returning.
            // Note this is in the same order as the StackContextFrame` struct.
            "pushq %rbx",
            "pushq %r11", // in lieu of rsp, push r11
            "pushq %rbp",
            "pushq %r12",
            "pushq %r13",
            "pushq %r14",
            "pushq %r15",
            "pushfq",
            // Load the address of the exit label.
            "leaq 1f(%rip), %rax",
            "pushq %rax",

            // The stack is the base pointer of the StackContextFrame struct, so we can move it to
            // rdi so that the rust handler treats it as the first parameter.
            "movq %rsp, %rdi",

            // stack frame setup
            "andq $~15, %rsp",
            // Call the rust handler:
            // - %rdi holds the first argument, the context pointer (moved from %rsp)
            // - %rsi, %rdx holds the 2nd/3rd argument of this function (and is not clobbered), so it's
            //   forwarded by default.
            "call {0}",
            "ud2", // save_context_save is noreturn; insert ud2 here for safety
            "1:",

            // rbp is restored here...
            // TODO: consider saving/restoring rbp in same way as the other callee saved regs
            "popq %rbp",
            "ret",
            sym save_context_save::<T>,
            options(att_syntax)
        )
    }

    unsafe { save_context_impl(slice_stack_pointer(stack), &raw mut ctx, &raw mut fwd) };

    // Need to forget here because they were moved into save_context_impl, despite the semantics
    // being a bit weird.
    forget(ctx);
    forget(fwd);
}
