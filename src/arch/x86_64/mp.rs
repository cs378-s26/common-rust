use core::arch::asm;

use core::sync::atomic::{AtomicU32, Ordering};

use alloc::boxed::Box;
use limine::mp::Cpu;
use spin::Once;
use x86::msr::IA32_FS_BASE;
use x86::{
    cpuid::CpuId,
    msr::{IA32_GS_BASE, wrmsr},
};

use crate::arch::x86_64::slice_stack_pointer;
use crate::arch::x86_64::tables::{
    GlobalDescriptorTable, InterruptDescriptorTable, InterruptStackTable,
};
use crate::arch::{TIMER_INTERRUPT_VECTOR, apic, tsc};
use crate::heap::aligned_slice;
use crate::{
    arch::x86_64::cpuid::Features,
    mp::{CORE_ID, CoreId, core_local, get_cpu_local_pointer_for},
    print::kprint,
};

static CPU_ID_PRINT: Once<()> = Once::new();

core_local! {
    pub LAPIC_ID: AtomicU32 = AtomicU32::new(0);
    IST: Once<InterruptStackTable> = Once::new();
    GDT: Once<GlobalDescriptorTable> = Once::new();
    IDT: Once<InterruptDescriptorTable> = Once::new();
}

// Gheith uses FSGSBASE instructions here, but the MSR is older and doesn't need to be enabled
pub fn init_cpu_local_ptr(core_id: CoreId) {
    let ptr = get_cpu_local_pointer_for(core_id);
    unsafe { wrmsr(IA32_GS_BASE, ptr) };
}

pub fn get_cpu_local_pointer() -> u64 {
    let mut val: u64;

    unsafe {
        asm!(
            "movq %gs:0, {}",
            lateout(reg) val,
            options(nostack, preserves_flags, pure, readonly, att_syntax),
        );
    }

    val
}

pub unsafe fn set_thread_local_pointer(base: *const u64) {
    unsafe { wrmsr(IA32_FS_BASE, base as u64) };
}

pub unsafe fn get_thread_local_pointer() -> u64 {
    let mut val: u64;

    unsafe {
        asm!(
            "movq %fs:0, {}",
            lateout(reg) val,
            options(nostack, preserves_flags, pure, readonly, att_syntax),
        );
    }

    val
}

pub unsafe fn initialize_core(cpu: &Cpu) {
    let id = CoreId(cpu.extra.load(Ordering::SeqCst) as usize);
    init_cpu_local_ptr(id);
    CORE_ID.replace(id);
    LAPIC_ID.store(cpu.lapic_id, Ordering::Relaxed);

    let cpu_id = CpuId::new();

    CPU_ID_PRINT.call_once(|| kprint!("{}", Features(&cpu_id)));

    fn allocate_sp(pages: usize) -> u64 {
        slice_stack_pointer(unsafe { &*Box::into_raw(aligned_slice(pages * 4096, 4096)) })
    }

    // stores the stacks we switch to when interrupts occur
    let ist = IST.call_once(|| InterruptStackTable {
        reserved0: 0,
        rsp0: 0,
        rsp1: 0,
        rsp2: 0,
        reserved1: 0,
        // allocate interrupt stacks
        ist1: allocate_sp(32),
        ist2: allocate_sp(32),
        ist3: allocate_sp(32),
        ist4: allocate_sp(32),
        ist5: allocate_sp(32),
        ist6: allocate_sp(32),
        ist7: allocate_sp(32),
        reserved2: 0,
        reserved3: 0,
        io_bp: 0,
    });

    let gdt = GDT.call_once(|| GlobalDescriptorTable::new(ist));
    let idt = IDT.call_once(InterruptDescriptorTable::new);

    unsafe { gdt.load() };
    unsafe { idt.load() };

    // we need to re-load the core local, becase the FS/GSBASE registers are really just references
    // to the "cached" segment base registers, which gets reset on descriptor reloads
    init_cpu_local_ptr(id);
    if !apic::enable_apic() {
        panic!("Failed to enable APIC");
    }
    apic::init_lapic();
    apic::disable_pic();

    // Calibrate timers once on BSP, store frequencies for all cores
    static TIMER_CALIBRATION: Once<(u64, u64)> = Once::new();
    let (_tsc_freq, apic_freq) = *TIMER_CALIBRATION.call_once(|| {
        let tsc_freq = tsc::calibrate_tsc_with_pit();

        let apic_freq =
            apic::calibrate_apic_timer_with_tsc(tsc_freq).expect("Failed to calibrate APIC timer");

        (tsc_freq, apic_freq)
    });

    {
        let timer_hz = 500;
        let initial_count = (apic_freq / timer_hz) as u32;
        apic::setup_timer(TIMER_INTERRUPT_VECTOR, initial_count, true);
    }
}
