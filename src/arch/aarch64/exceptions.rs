use crate::{
    arch::{Arch, IrqStateTrait},
    mp::CORE_ID,
    print::kprintln,
};
use core::arch::{asm, global_asm};
use core::fmt;

global_asm!(include_str!("exception.s"));

/// The exception context as it is stored on the stack on exception entry.
#[repr(C)]
struct ExceptionContext {
    gpr: [u64; 30],
    lr: u64,
    elr_el1: u64,
    spsr_el1: u64,
    esr_el1: u64,
}

/// Prints verbose information about the exception and then panics.
fn default_exception_handler(exc: &mut ExceptionContext) {
    let exception_class = (exc.esr_el1 >> 26) & 0b111111;

    kprintln!("core {} exc class: {:x}", CORE_ID.get(), exception_class);
    if exception_class == 0x15 {
        kprintln!("SVC");
        exc.elr_el1 += 4;
        exc.spsr_el1 &= !(1 << 7); // clear IRQ mask.
        return;
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
    kprintln!("Current elx synchronous");
    default_exception_handler(e);
}

#[unsafe(no_mangle)]
extern "C" fn current_elx_irq(e: &mut ExceptionContext) {
    kprintln!("Current elx irq");
    let intid: u64;
    unsafe {
        core::arch::asm!(
            "mrs {}, ICC_IAR1_EL1",
            out(reg) intid,
        );
    }

    // mask out the INTID (bits [23:0], top bits are affinity routing)
    let intid = intid & 0xFF_FFFF;

    match intid {
        30 => {
            kprintln!("physical timer ticked");
            unsafe {
                core::arch::asm!(
                    "msr CNTP_TVAL_EL0, {}",
                    in(reg) 62_500_000u64,
                );
            }
        }
        1023 => { /* spurious interrupt, ignore */ }
        _ => panic!("unexpected INTID: {}", intid),
    }

    // signal EOI — "I'm done handling this"
    unsafe {
        core::arch::asm!(
            "msr ICC_EOIR1_EL1, {}",
            in(reg) intid,
        );
    }
}

#[unsafe(no_mangle)]
extern "C" fn current_elx_serror(e: &mut ExceptionContext) {
    default_exception_handler(e);
}

// Usermode

#[unsafe(no_mangle)]
extern "C" fn el0_sync(e: &mut ExceptionContext) {
    default_exception_handler(e);
}

#[unsafe(no_mangle)]
extern "C" fn el0_irq(e: &mut ExceptionContext) {
    default_exception_handler(e);
}

#[unsafe(no_mangle)]
extern "C" fn el0_serror(e: &mut ExceptionContext) {
    default_exception_handler(e);
}

#[unsafe(no_mangle)]
extern "C" fn unimplemented(e: &mut ExceptionContext) {
    default_exception_handler(e);
}

//--------------------------------------------------------------------------------------------------
// Public Code
//--------------------------------------------------------------------------------------------------

/// The processing element's current privilege level.
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
    kprintln!("{} enabling exceptions", CORE_ID.get());
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

    let mut vector_base_addr = unsafe { &exception_vector_table as *const _ as u64 };

    unsafe {
        asm!("msr VBAR_EL1, {}", in(reg) (vector_base_addr as u64));
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
        for (i, reg) in self.gpr.iter().enumerate() {
            write!(f, "  x{:<2}: {:#018x}", i, reg)?;
            if i % 2 == 1 {
                writeln!(f)?;
            } else {
                write!(f, "    ")?;
            }
        }
        writeln!(f, "  x30: {:#018x} (LR)", self.lr)?;

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
