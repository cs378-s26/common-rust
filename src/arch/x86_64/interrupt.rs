use core::arch::naked_asm;
use core::sync::atomic::{AtomicU64, Ordering};

use x86::{
    bits64::rflags::{self, RFlags},
    controlregs::cr2,
    irq,
};

use super::apic;
use crate::arch::{Arch, IrqStateTrait};

static TIMER_TICKS: AtomicU64 = AtomicU64::new(0);

pub fn timer_ticks() -> u64 {
    TIMER_TICKS.load(Ordering::Relaxed)
}

#[repr(transparent)]
#[derive(PartialEq, Eq, Clone, Copy)]
pub struct IrqState(bool);

impl IrqStateTrait for IrqState {
    type Arch = Arch;

    #[inline(always)]
    fn save() -> IrqState {
        IrqState(rflags::read().contains(RFlags::FLAGS_IF))
    }

    fn is_masked(&self) -> bool {
        !self.0
    }
}

impl IrqState {
    #[inline(always)]
    pub fn restore(self) {
        if self.0 {
            unsafe { irq::enable() };
        } else {
            unsafe { irq::disable() };
        }
    }

    pub fn is_irq_enabled(self) -> bool {
        self.0
    }
}

pub fn irq_is_enabled() -> bool {
    rflags::read().contains(RFlags::FLAGS_IF)
}

#[inline(always)]
pub unsafe fn disable() {
    unsafe { irq::disable() };
}

#[inline(always)]
pub unsafe fn enable() {
    unsafe { irq::enable() };
}

/// Layout matches what irq_handler_t0 pushes onto the stack (low to high):
/// r15, r14, r13, r12, r11, r10, r9, r8, rdi, rsi, rbx, rdx, rcx, rax, rbp
/// id, err, rip, cs, rflags, rsp, ss
#[repr(C)]
pub struct InterruptContext {
    pub regs: [u64; 14],
    pub rbp: u64, // For preemptive context restore.
    pub id: u64,
    pub err: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

// IDT magic

const fn error_code_offset(int_no: u8) -> u64 {
    if int_no == 8 || (10..=14).contains(&int_no) || int_no == 17 || int_no == 21 {
        0
    } else {
        8
    }
}

#[unsafe(naked)]
pub(super) unsafe extern "C" fn irq_handler_entry<const I: u8>() -> ! {
    naked_asm!(
        // required for ABI reasons
        "cld",

        // normalize the stack frame: [int#, ec]
        "subq ${}, %rsp",
        "pushq ${}",
        "jmp {}",

        options(att_syntax),
        const error_code_offset(I),
        const I,
        sym irq_handler_t0
    )
}

#[unsafe(naked)]
pub unsafe extern "C" fn irq_handler_t0() -> ! {
    naked_asm!(
        "pushq %rbp",
        "pushq %rax",
        "pushq %rcx",
        "pushq %rdx",
        "pushq %rbx",
        "pushq %rsi",
        "pushq %rdi",
        "pushq %r8",
        "pushq %r9",
        "pushq %r10",
        "pushq %r11",
        "pushq %r12",
        "pushq %r13",
        "pushq %r14",
        "pushq %r15",

        // point to top of stack (1st arg: InterruptContext*)
        "movq %rsp, %rdi",

        // simulate the call frame
        "pushq $0",
        "pushq %rbp",
        "movq %rsp, %rbp",

        // align stack
        "andq $~15, %rsp",

        // invoke
        "call {}",

        "movq %rbp, %rsp",
        "popq %rbp",
        "addq $8, %rsp",

        "popq %r15",
        "popq %r14",
        "popq %r13",
        "popq %r12",
        "popq %r11",
        "popq %r10",
        "popq %r9",
        "popq %r8",
        "popq %rdi",
        "popq %rsi",
        "popq %rbx",
        "popq %rdx",
        "popq %rcx",
        "popq %rax",
        "popq %rbp",

        "addq $16, %rsp",
        "iretq",
        options(att_syntax),
        sym irq_handler_t1
    );
}

pub const TIMER_INTERRUPT_VECTOR: u8 = 0x20;
pub const IPI_WAKE_VECTOR: u8 = 0x21;

pub extern "C" fn timer_interrupt_handler(ctx: &InterruptContext) {
    TIMER_TICKS.fetch_add(1, Ordering::Relaxed);
    apic::eoi();

    unsafe { crate::thread::preempt_from_interrupt(ctx) };
}

pub extern "C" fn ipi_wake_handler(_ctx: &InterruptContext) {
    apic::eoi();
}

unsafe extern "C" fn irq_handler_t1(addr: *mut InterruptContext) {
    let context = unsafe { &*addr };

    match context.id as u8 {
        TIMER_INTERRUPT_VECTOR => timer_interrupt_handler(context),
        IPI_WAKE_VECTOR => ipi_wake_handler(context),
        _ => panic!(
            "Unhandled interrupt #{}: err={}, cr2={:x}",
            context.id,
            context.err,
            unsafe { cr2() }
        ),
    }
}
