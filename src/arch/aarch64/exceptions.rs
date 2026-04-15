use core::{
    arch::{asm, global_asm},
    fmt,
};

use super::{context::GPRegisters, interrupt::InterruptContext};
use crate::{
    arch::aarch64::gic,
    event::{Event, push_event},
    memory::virtual_memory::PageFaultConditions,
    mp::CORE_ID,
    print::kprintln,
    thread::{IDLE, block_to_idle, preempt_to_idle, suspend_to_thread, this_thread},
};

global_asm!(include_str!("exception.s"));

// TODO: use bitflags or smth

// TODO: do we need a core local stack for temporary interrupts?
// context switching happens on a context switching stack. this can
// act like a core local stack for us.

// docs for all this here:
// https://developer.arm.com/documentation/111107/2025-12/AArch64-Registers/ESR-EL1--Exception-Syndrome-Register--EL1-
const SVC: u64 = 0b010101; // SVC instruction from AArch64
const INSTRUCTION_ABORT: u64 = 0b100001; // Instruction Abort from same EL
const INSTRUCTION_ABORT_LOWER: u64 = 0b100000; // Instruction Abort from lower EL
const DATA_ABORT: u64 = 0b100101; // Data Abort from same EL
const DATA_ABORT_LOWER: u64 = 0b100100; // Data Abort from lower EL
const ISS_MASK: u64 = 0x1FFFFFF; // Instruction Specific Syndrome mask

// Data Abort ISS bits from ESR_EL1.
const DATA_ABORT_DFSC_MASK: u64 = 0b11_1111;
const DATA_ABORT_WNR: u64 = 1 << 6;
const DATA_ABORT_S1PTW: u64 = 1 << 7;
const DATA_ABORT_FNV: u64 = 1 << 10;

const DFSC_TRANSLATION_FAULT_L0: u64 = 0b000100;
const DFSC_TRANSLATION_FAULT_L3: u64 = 0b000111;
const DFSC_ACCESS_FLAG_FAULT_L0: u64 = 0b001000;
const DFSC_ACCESS_FLAG_FAULT_L3: u64 = 0b001011;
const DFSC_PERMISSION_FAULT_L0: u64 = 0b001100;
const DFSC_PERMISSION_FAULT_L3: u64 = 0b001111;

/// The exception context as it is stored on the stack on exception entry.
#[repr(C)]
struct ExceptionContext {
    gpr: GPRegisters,
    elr_el1: u64,
    spsr_el1: u64,
    esr_el1: u64,
    sp: u64,
    _pad: u64, // padding to make the size a multiple of 16 bytes for alignment, see SAVE_REGS in exception.s
}

/// Prints verbose information about the exception and then panics.
fn default_exception_handler(exc: &mut ExceptionContext) {
    let exception_class = (exc.esr_el1 >> 26) & 0b111111;

    if exception_class == SVC {
        // TODO write an architecture agnostic system call trap_frame that ExceptionContext implements so system calls can be passed this and just work
        // system_call_handler(exc);
        let syscall_id = exc.gpr.regs[8];

        this_thread()
            .process
            .get()
            .unwrap()
            .exit_code
            .set(syscall_id);
        suspend_to_thread(IDLE.get().unwrap().clone());

        return;
    } else if exception_class == INSTRUCTION_ABORT || exception_class == INSTRUCTION_ABORT_LOWER {
        // TODO: iss bits for instruction abort as well
        if exception_class == INSTRUCTION_ABORT_LOWER {
            page_fault_handler(exc, exception_class);
        } else {
            kprintln!("Instruction abort at address {:#018x}", exc.elr_el1);
        }
    } else if exception_class == DATA_ABORT || exception_class == DATA_ABORT_LOWER {
        page_fault_handler(exc, exception_class);
    } else {
        kprintln!("Unhandled exception class: {:x}", exception_class);
    }

    panic!(
        "Exception on core {}!\n\n\
        {}",
        CORE_ID.get(),
        exc
    );
}

//------------------------------------------------------------------------------
// Current, ELx
//------------------------------------------------------------------------------

#[unsafe(no_mangle)]
extern "C" fn current_elx_synchronous(e: &mut ExceptionContext) {
    default_exception_handler(e);
}

fn page_fault_handler(e: &mut ExceptionContext, exception_class: u64) {
    let far_el1: u64;

    unsafe {
        asm!("mrs {}, FAR_EL1", out(reg) far_el1);
    }
    let iss = e.esr_el1 & ISS_MASK;

    if let Some(cause) = page_fault_cause(exception_class, iss) {
        push_event(
            Event::PageFault {
                cause,
                address: far_el1 as usize,
                thread: this_thread(),
            },
            CORE_ID.get(),
        );

        let interrupt_context = InterruptContext {
            gpr: e.gpr,
            sp: e.sp,
            pc: e.elr_el1,
            spsr: e.spsr_el1,
        };

        unsafe {
            block_to_idle(&interrupt_context);
        }
    } else {
        kprintln!("Data abort is not a page fault: ISS={:#010x}", iss);
    }
}

#[allow(unused_variables)]
fn timer_interrupt_handler(e: &mut ExceptionContext) {
    gic::timer_reset_interval();
    gic::inc_timer_ticks();
    let ticks = gic::timer_ticks();

    let interrupt_context = InterruptContext {
        gpr: e.gpr,
        sp: e.sp,
        pc: e.elr_el1,
        spsr: e.spsr_el1,
    };

    gic::eoi(30);
    unsafe {
        preempt_to_idle(&interrupt_context);
    }
}

#[unsafe(no_mangle)]
extern "C" fn current_elx_irq(e: &mut ExceptionContext) {
    let intid = gic::get_intid_ack_irq();

    match intid {
        30 => timer_interrupt_handler(e),
        1023 => {
            kprintln!("Spurrious interrupt");
            // No EOI for spurious interrupts
            return;
        }
        _ => panic!("unexpected INTID: {}", intid),
    }

    gic::eoi(intid);
}

// Usermode

#[unsafe(no_mangle)]
extern "C" fn el0_sync(e: &mut ExceptionContext) {
    default_exception_handler(e);
}

#[unsafe(no_mangle)]
extern "C" fn el0_irq(e: &mut ExceptionContext) {
    current_elx_irq(e);
}

#[unsafe(no_mangle)]
extern "C" fn unimplemented(e: &mut ExceptionContext) {
    kprintln!("Hit unimplemented exception vector!");

    panic!(
        "Exception on core {}!\n\n\
        {}",
        CORE_ID.get(),
        e
    );
}

//--------------------------------------------------------------------------------------------------
// Public Code
//--------------------------------------------------------------------------------------------------

/// The processing element's current privilege level.
#[allow(dead_code)]
pub fn current_privilege_level() -> &'static str {
    let mut el: u64;
    unsafe {
        asm!("mrs {}, CurrentEL", out(reg) el);
    }
    el = (el >> 2) & 0b11;

    match el {
        2 => "EL2",
        1 => "EL1",
        0 => "EL0",
        _ => "Unknown",
    }
}

pub fn init_exceptions() {
    // set up a stack per EL and move stack pointer to work with the EL1 stack
    unsafe {
        core::arch::asm!(
            "mov x0, sp",
            "msr SPSel, #1",
            "mov sp, x0",
            out("x0") _,
        );
    }
    unsafe extern "C" {
        unsafe static exception_vector_table: u8;
    }

    let vector_base_addr = unsafe { &exception_vector_table as *const _ as u64 };

    unsafe {
        asm!("msr VBAR_EL1, {}", in(reg) (vector_base_addr));
    }
    kprintln!("{} enabled exceptions", CORE_ID.get());
}

pub fn dump_core_state(label: &str) {
    let mut sp: u64;
    let mut lr: u64;
    let mut tpidr_el1: u64;
    let mut tpidr_el0: u64;
    let mut spsr_el1: u64;
    let mut cpacr_el1: u64;
    let mut x0: u64;
    let mut x1: u64;

    unsafe {
        core::arch::asm!(
            "mov {sp_val}, sp",
            "mov {lr_val}, x30",
            "mrs {tpidr1}, tpidr_el1",
            "mrs {tpidr0}, tpidr_el0",
            "mrs {spsr}, spsr_el1",
            "mrs {cpacr}, cpacr_el1",
            "mov {x0_val}, x0",
            "mov {x1_val}, x1",
            sp_val = out(reg) sp,
            lr_val = out(reg) lr,
            tpidr1 = out(reg) tpidr_el1,
            tpidr0 = out(reg) tpidr_el0,
            spsr = out(reg) spsr_el1,
            cpacr = out(reg) cpacr_el1,
            x0_val = out(reg) x0,
            x1_val = out(reg) x1,
            options(nomem, nostack, preserves_flags),
        );
    }

    kprintln!("=== core state [{}] on core {} ===", label, CORE_ID.get());
    kprintln!("  sp        = {:#018x}", sp);
    kprintln!("  lr (x30)  = {:#018x}", lr);
    kprintln!("  tpidr_el1 = {:#018x}", tpidr_el1);
    kprintln!("  tpidr_el0 = {:#018x}", tpidr_el0);
    kprintln!("  spsr_el1  = {:#018x}", spsr_el1);
    kprintln!("  cpacr_el1 = {:#018x}", cpacr_el1);
    kprintln!("  x0        = {:#018x}", x0);
    kprintln!("  x1        = {:#018x}", x1);
}

fn page_fault_cause(exception_class: u64, iss: u64) -> Option<PageFaultConditions> {
    let dfsc = iss & DATA_ABORT_DFSC_MASK;

    // if it's a translation fault, present bit should be set to zero, otherwise the page must have been present
    let mut cause = if is_translation_fault(dfsc) {
        PageFaultConditions::empty()
    } else if is_access_or_permission_fault(dfsc) {
        PageFaultConditions::PRESENT
    } else {
        return None;
    };

    if (iss & DATA_ABORT_WNR) != 0 {
        cause.insert(PageFaultConditions::WRITE);
    }
    if exception_class == DATA_ABORT_LOWER {
        cause.insert(PageFaultConditions::USER);
    }

    // this means walking page tables caused a fault (besides for causes above) or Far Not Valid,
    // meaning we don't have an address for the fault
    if (iss & (DATA_ABORT_S1PTW | DATA_ABORT_FNV)) != 0 {
        cause.insert(PageFaultConditions::CORRUPT);
    }

    Some(cause)
}

// look into the dfsc code to find the cause of the data abort. There are different bit codes for
// each level of page table, so we check if it's inbetween L0 and L3
fn is_translation_fault(dfsc: u64) -> bool {
    (DFSC_TRANSLATION_FAULT_L0..=DFSC_TRANSLATION_FAULT_L3).contains(&dfsc)
}

fn is_access_or_permission_fault(dfsc: u64) -> bool {
    (DFSC_ACCESS_FLAG_FAULT_L0..=DFSC_ACCESS_FLAG_FAULT_L3).contains(&dfsc)
        || (DFSC_PERMISSION_FAULT_L0..=DFSC_PERMISSION_FAULT_L3).contains(&dfsc)
}

impl fmt::Display for ExceptionContext {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "ELR_EL1: {:#018x}", self.elr_el1)?;
        writeln!(f, "SPSR_EL1: {:#010x}", self.spsr_el1)?;
        writeln!(f, "ESR_EL1: {:#010x}", self.esr_el1)?;

        let _ = writeln!(f, "coreid {}", CORE_ID.get());

        // Decode ESR_EL1 for useful information
        let exception_class = (self.esr_el1 >> 26) & 0b111111;
        let iss = self.esr_el1 & 0x1FFFFFF; // Instruction Specific Syndrome

        writeln!(
            f,
            "\nException Class: {:#08b} ({})",
            exception_class,
            Self::exception_class_to_str(exception_class)
        )?;
        writeln!(f, "ISS: {:#x}", iss)?;

        // Get FAR_EL1 for memory faults
        if matches!(exception_class, 0b100000..=0b100101) {
            let far_el1: u64;
            unsafe {
                asm!("mrs {}, FAR_EL1", out(reg) far_el1);
            }
            writeln!(f, "FAR_EL1: {:#018x}", far_el1)?;
        }

        // Print general purpose registers
        writeln!(f, "\nGeneral Purpose Registers:")?;
        for (i, reg) in self.gpr.regs.iter().enumerate() {
            write!(f, "  x{:<2}: {:#018x}", i, reg)?;
            if i % 2 == 1 {
                writeln!(f)?;
            } else {
                write!(f, "    ")?;
            }
        }

        Ok(())
    }
}

impl ExceptionContext {
    fn exception_class_to_str(ec: u64) -> &'static str {
        match ec {
            0b000000 => "Unknown reason",
            0b000001 => "Trapped WFI or WFE",
            0b000011 => "Trapped MCR or MRC (CP15)",
            0b000100 => "Trapped MCRR or MRRC (CP15)",
            0b000101 => "Trapped MCR or MRC (CP14)",
            0b000110 => "Trapped LDC or STC (CP14)",
            0b000111 => "Trapped access to SVE/SIMD/FP",
            0b001100 => "Trapped MRRC (CP14)",
            0b001101 => "Branch Target Exception",
            0b001110 => "Illegal Execution State",
            0b010001 => "SVC instruction from AArch32",
            0b010101 => "SVC instruction from AArch64",
            0b011000 => "Trapped MSR, MRS or System instruction",
            0b011001 => "Trapped access to SVE",
            0b011100 => "Trapped pointer authentication",
            0b100000 => "Instruction Abort from lower EL",
            0b100001 => "Instruction Abort from same EL",
            0b100010 => "PC alignment fault",
            0b100100 => "Data Abort from lower EL",
            0b100101 => "Data Abort from same EL",
            0b100110 => "SP alignment fault",
            0b101000 => "Trapped floating-point (AArch32)",
            0b101100 => "Trapped floating-point (AArch64)",
            0b101111 => "SError interrupt",
            0b110000 => "Breakpoint from lower EL",
            0b110001 => "Breakpoint from same EL",
            0b110010 => "Software Step from lower EL",
            0b110011 => "Software Step from same EL",
            0b110100 => "Watchpoint from lower EL",
            0b110101 => "Watchpoint from same EL",
            0b111000 => "BKPT instruction (AArch32)",
            0b111100 => "BRK instruction (AArch64)",
            _ => "Reserved/Unknown",
        }
    }
}
