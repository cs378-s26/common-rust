use core::{
    arch::naked_asm,
    mem::forget,
    ptr::{self},
};

use spin::MutexGuard;
use x86::{Ring, bits64::rflags::RFlags, segmentation::SegmentSelector};

use crate::arch::x86_64::{slice_stack_pointer, tables::GlobalDescriptorTable};

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

    // 16 bit, extended to 64
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
            rflags: RFlags::FLAGS_IF,
            cs: Default::default(),
            ss: Default::default(),
        }
    }
}

#[unsafe(naked)]
// rdi, rsi, rdx, rcx, r8, r9
unsafe extern "C" fn jump_to_context(
    buf: *const GPRegisters,
    ss: u64,
    rsp: u64,
    rflags: u64,
    cs: u64,
    rip: u64,
) -> ! {
    // TODO: we need to switch ds here
    // maybe also fs
    // and maybe everything else
    // oh well it's up to John Userspace to do this
    naked_asm!(
        "pushq %rsi",
        "pushq %rdx",
        "pushq %rcx",
        "pushq %r8",
        "pushq %r9",
        // oh god this routine gives me flashbacks
        "movq (0 * 8)(%rdi), %rax",
        "movq (1 * 8)(%rdi), %rbx",
        "movq (2 * 8)(%rdi), %rcx",
        "movq (3 * 8)(%rdi), %rdx",
        "movq (4 * 8)(%rdi), %rsi",
        // "movq (5 * 8)(%rdi), %rdi", MOVED
        "movq (6 * 8)(%rdi), %rbp",
        // "movq (7 * 8)(%rdi), %rsp", BAD BAD BAD don't load sp
        "movq (8 * 8)(%rdi), %r8",
        "movq (9 * 8)(%rdi), %r9",
        "movq (10 * 8)(%rdi), %r10",
        "movq (11 * 8)(%rdi), %r11",
        "movq (12 * 8)(%rdi), %r12",
        "movq (13 * 8)(%rdi), %r13",
        "movq (14 * 8)(%rdi), %r14",
        "movq (15 * 8)(%rdi), %r15",
        // don't clobber registers!
        "movq (5 * 8)(%rdi), %rdi",
        // why use iretq? because of the woke left. just kidding. it makes handling DPL easier once you get userspace :D
        "iretq",
        options(att_syntax)
    )
}

impl Context {
    pub fn jump_to(&self) -> ! {
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

    pub fn setup_kthread_context(&mut self) {
        self.cs = SegmentSelector::new(GlobalDescriptorTable::CS, Ring::Ring0)
            .bits()
            .into();

        self.ss = SegmentSelector::new(GlobalDescriptorTable::DS, Ring::Ring0)
            .bits()
            .into();
    }

    pub fn setup_for_call<T>(
        &mut self,
        stack: &[u8],
        function: unsafe extern "C" fn(*mut T) -> !,
        data: *mut T,
    ) {
        self.setup_kthread_context();

        self.rip = function as usize as u64;
        self.gp.rdi = data as u64;
        self.gp.rsp = slice_stack_pointer(stack);
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

// TODO: this routine is most definitely NOT SAFE and has a bunch of undefined behavior
// I should attempt to reduce the amount of UB here, but this is a nontrivial question
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
        let mut ctx:  MutexGuard<'static, Context> = unsafe { ptr::read(ctx) };

        ctx.gp.rbx = frame.rbx;
        ctx.gp.rsp = frame.rsp;
        ctx.gp.rbp = frame.rbp;
        ctx.gp.r12 = frame.r12;
        ctx.gp.r13 = frame.r13;
        ctx.gp.r14 = frame.r14;
        ctx.gp.r15 = frame.r15;
        ctx.rip = frame.rip;
        ctx.rflags = RFlags::from_bits(frame.rflags).unwrap();

        // at this point, it is safe to resume the current thread on any other core
        // all relevant context variables are moved onto the new temporary stack
        drop(ctx);

        fwd();
    }

    #[unsafe(naked)]
    unsafe extern "C" fn save_context_impl<T: FnOnce() -> !>(
        stack: u64,
        ctx: *mut MutexGuard<'static, Context>,
        fwd: *mut T,
    ) {
        // Functions preserve the registers rbx, rsp, rbp, r12, r13, r14, and r15
        // so we should store them in the context, and jump to the handler
        //
        // strictly speaking, this is probably not ABI compliant
        naked_asm!(
            "pushq %rbp",
            "movq %rsp, %rbp",

            // use r11 as a scratch register to hold rsp
            "movq %rsp, %r11",
            // set rsp = stack, switch off the main call stack
            "movq %rdi, %rsp",

            // push things onto the *new* stack
            "pushq %rbx",
            "pushq %r11", // in lieu of rsp, push r11
            "pushq %rbp",
            "pushq %r12",
            "pushq %r13",
            "pushq %r14",
            "pushq %r15",
            "pushfq",
            "leaq 1f(%rip), %rax",
            "pushq %rax",

            "movq %rsp, %rdi",

            // stack frame setup
            "andq $~15, %rsp",
            "call {0}",
            "ud2",
            "1:",

            "popq %rbp",
            "ret",
            sym save_context_save::<T>,
            options(att_syntax)
        )
    }

    unsafe { save_context_impl(slice_stack_pointer(stack), &raw mut ctx, &raw mut fwd) };

    // need to forget here because they were moved into save_context_impl, despite the semantics
    // being a bit weird
    forget(ctx);
    forget(fwd);
}
