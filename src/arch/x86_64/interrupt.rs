use core::{arch::naked_asm, iter::empty};
use crate::virtual_memory::{PageFaultConditions, handle_page_fault};

use x86::{
    bits64::rflags::{self, RFlags},
    controlregs::cr2,
    irq,
};
use x86_64::structures::idt::PageFaultErrorCode;

use crate::arch::{Arch, IrqStateTrait};

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

#[inline(always)]
pub unsafe fn disable() {
    unsafe { irq::disable() };
}

#[inline(always)]
pub unsafe fn enable() {
    unsafe { irq::enable() };
}

#[repr(C)]
struct InterruptContext {
    regs: [u64; 14],
    id: u64,
    err: u64,
    rip: u64,
    cs: u64,
    rflags: u64,
    rsp: u64,
    ss: u64,
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

        // point to top of stack
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

        "addq $16, %rsp",
        "iretq",
        options(att_syntax),
        sym irq_handler_t1
    );
}

unsafe extern "C" fn irq_handler_t1(addr: *mut InterruptContext) {
    let context = unsafe { &*addr };
    // we are in a very fragile context here
    // we should probably tell the threading module that we are no longer in a thread...
    // TODO: implement that logic :D

    match context.id {
        14 => {
            if let Some(code) = PageFaultErrorCode::from_bits(context.err) {
                // seems like kind of a lot of overhead for interface translation...
                let mut cause = PageFaultConditions::empty();
                if code.contains(PageFaultErrorCode::PROTECTION_VIOLATION) {cause.insert(PageFaultConditions::PRESENT);}
                if code.contains(PageFaultErrorCode::CAUSED_BY_WRITE) {cause.insert(PageFaultConditions::WRITE);}
                if code.contains(PageFaultErrorCode::USER_MODE) {cause.insert(PageFaultConditions::USER);}
                if code.contains(PageFaultErrorCode::MALFORMED_TABLE) {cause.insert(PageFaultConditions::CORRUPT);}
                if code.contains(PageFaultErrorCode::INSTRUCTION_FETCH) {cause.insert(PageFaultConditions::FETCH);}
                handle_page_fault(cause, unsafe {cr2()});
            } else {
                panic!("hi: {} #{}, cr2={}", context.err, context.id, unsafe {
                    cr2()
                });
            }
        }
        _ => {panic!("hi: {} #{}, cr2={}", context.err, context.id, unsafe {
            cr2()
        });}
    }

}

