use crate::syscall::{syscall_handler, SyscallContext};
use crate::virtual_memory::{PageFaultConditions, handle_page_fault};
use core::arch::naked_asm;
use core::sync::atomic::{AtomicU64, Ordering};

use x86::controlregs::cr2;
use x86_64::structures::idt::PageFaultErrorCode;

use super::apic;

static TIMER_TICKS: AtomicU64 = AtomicU64::new(0);

pub fn timer_ticks() -> u64 {
    TIMER_TICKS.load(Ordering::Relaxed)
}

#[repr(C)]
pub struct InterruptContext {
    // TODO just make the below explicit
    pub regs: [u64; 14],     // general-purpose registers
    pub r15: u64,
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
unsafe extern "C" fn irq_handler_t0() -> ! {
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

    unsafe { crate::thread::preempt_to_idle(ctx) };
}

pub extern "C" fn ipi_wake_handler(_ctx: &InterruptContext) {
    apic::eoi();
}

// this is all entirely by convention, but it's nice to follow the C ABI
impl SyscallContext for InterruptContext {
    fn syscall_number(&self) -> usize {
        self.regs[13] as usize // rax
    }
    fn arg0(&self) -> usize {
        self.regs[8] as usize // rdi
    }
    fn arg1(&self) -> usize {
        self.regs[9] as usize // rsi
    }
    fn arg2(&self) -> usize {
        self.regs[11] as usize // rdx
    }
    fn arg3(&self) -> usize {
        self.regs[12] as usize // rcx
    }
    fn arg4(&self) -> usize {
        self.regs[7] as usize // r8
    }
    fn arg5(&self) -> usize {
        self.regs[6] as usize // r9
    }
    /// # Safety
    /// Can only be called after all calls to syscall_number
    fn set_return_value(&mut self, ret: usize) {
        self.regs[13] = ret as u64;
    }
}

unsafe extern "C" fn irq_handler_t1(addr: *mut InterruptContext) {
    let context = unsafe { &mut *addr };
    match context.id as u8 {
        14 => {
            if let Some(code) = PageFaultErrorCode::from_bits(context.err) {
                // seems like kind of a lot of overhead for interface translation...
                let mut cause = PageFaultConditions::empty();
                if code.contains(PageFaultErrorCode::PROTECTION_VIOLATION) {
                    cause.insert(PageFaultConditions::PRESENT);
                }
                if code.contains(PageFaultErrorCode::CAUSED_BY_WRITE) {
                    cause.insert(PageFaultConditions::WRITE);
                }
                if code.contains(PageFaultErrorCode::USER_MODE) {
                    cause.insert(PageFaultConditions::USER);
                }
                if code.contains(PageFaultErrorCode::MALFORMED_TABLE) {
                    cause.insert(PageFaultConditions::CORRUPT);
                }
                if code.contains(PageFaultErrorCode::INSTRUCTION_FETCH) {
                    cause.insert(PageFaultConditions::FETCH);
                }
                handle_page_fault(cause, unsafe { cr2() });
            } else {
                panic!("hi: {} #{}, cr2={}", context.err, context.id, unsafe {
                    cr2()
                });
            }
        }
        TIMER_INTERRUPT_VECTOR => timer_interrupt_handler(context),
        IPI_WAKE_VECTOR => ipi_wake_handler(context),
        0x42 => {
            syscall_handler(&mut *context);
        }
        _ => panic!(
            "Unhandled interrupt #{}: err={}, cr2={:x}",
            context.id,
            context.err,
            unsafe { cr2() }
        ),
    }
}
