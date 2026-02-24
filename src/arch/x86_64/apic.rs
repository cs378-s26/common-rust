use core::arch::asm;

use x86::io::outb;
use x86::msr::{rdmsr, wrmsr};

use super::tsc::read_tsc;

const X2APIC_MSR_BASE: u32 = 0x800;

mod x2regs {
    pub const ID: u32 = 0x02;
    pub const TPR: u32 = 0x08;
    pub const EOI: u32 = 0x0B;
    pub const SIVR: u32 = 0x0F;
    pub const ESR: u32 = 0x28;
    pub const TIMER_LVT: u32 = 0x32;
    pub const TIMER_INITIAL_COUNT: u32 = 0x38;
    pub const TIMER_CURRENT_COUNT: u32 = 0x39;
    pub const TIMER_DIVIDE_CONFIG: u32 = 0x3E;
    pub const LINT0_LVT: u32 = 0x35;
    pub const LINT1_LVT: u32 = 0x36;
    pub const ERROR_LVT: u32 = 0x37;
}

const IA32_APIC_BASE_MSR: u32 = 0x1B;
const APIC_BASE_ENABLE: u64 = 1 << 11;
const APIC_BASE_X2APIC: u64 = 1 << 10;
const SIVR_APIC_ENABLED: u64 = 0x100;
const SPURIOUS_VECTOR: u64 = 0xFF;

pub fn x2apic_supported() -> bool {
    let ecx: u32;
    unsafe {
        asm!(
            "push rbx",
            "mov eax, 1",
            "xor ecx, ecx",
            "cpuid",
            "pop rbx",
            out("ecx") ecx,
            out("eax") _,
            out("edx") _,
        );
    }
    (ecx & (1 << 21)) != 0
}

pub fn x2apic_enabled() -> bool {
    unsafe {
        let base = rdmsr(IA32_APIC_BASE_MSR);
        (base & APIC_BASE_X2APIC) != 0
    }
}

#[inline]
unsafe fn x2apic_read(reg: u32) -> u64 {
    unsafe { rdmsr(X2APIC_MSR_BASE + reg) }
}

#[inline]
unsafe fn x2apic_write(reg: u32, value: u64) {
    unsafe { wrmsr(X2APIC_MSR_BASE + reg, value) };
}

pub fn enable_x2apic() -> bool {
    if !x2apic_supported() {
        return false;
    }
    unsafe {
        let mut base = rdmsr(IA32_APIC_BASE_MSR);
        base |= APIC_BASE_ENABLE | APIC_BASE_X2APIC;
        wrmsr(IA32_APIC_BASE_MSR, base);
    }
    true
}

pub fn get_lapic_id() -> u32 {
    unsafe { x2apic_read(x2regs::ID) as u32 }
}

pub fn init_lapic() {
    unsafe {
        x2apic_write(x2regs::SIVR, SIVR_APIC_ENABLED | SPURIOUS_VECTOR);
        x2apic_write(x2regs::TPR, 0);
        x2apic_write(x2regs::ESR, 0);
        x2apic_write(x2regs::TIMER_LVT, 0x10000);
        x2apic_write(x2regs::LINT0_LVT, 0x10000);
        x2apic_write(x2regs::LINT1_LVT, 0x10000);
        x2apic_write(x2regs::ERROR_LVT, 0x10000);
    }
}

pub fn eoi() {
    unsafe {
        x2apic_write(x2regs::EOI, 0);
    }
}

pub fn disable_pic() {
    unsafe {
        outb(0x21, 0xFF);
        outb(0xA1, 0xFF);
    }
}

pub fn setup_timer(vector: u8, initial_count: u32, periodic: bool) {
    unsafe {
        x2apic_write(x2regs::TIMER_DIVIDE_CONFIG, 0x0B);

        let mut lvt = vector as u64;
        if periodic {
            lvt |= 0x20000;
        }
        x2apic_write(x2regs::TIMER_LVT, lvt);
        x2apic_write(x2regs::TIMER_INITIAL_COUNT, initial_count as u64);
    }
}

pub fn stop_timer() {
    unsafe {
        x2apic_write(x2regs::TIMER_INITIAL_COUNT, 0);
        x2apic_write(x2regs::TIMER_LVT, 0x10000);
    }
}

pub fn read_timer() -> u32 {
    unsafe { x2apic_read(x2regs::TIMER_CURRENT_COUNT) as u32 }
}

pub fn calibrate_apic_timer_with_tsc(tsc_freq_hz: u64) -> Option<u64> {
    unsafe {
        stop_timer();
        x2apic_write(x2regs::TIMER_DIVIDE_CONFIG, 0x0B);
        x2apic_write(x2regs::TIMER_LVT, 0x10000 | 0xFF);
        x2apic_write(x2regs::TIMER_INITIAL_COUNT, 0xFFFFFFFF);

        let calibration_time_ms = 10;
        let tsc_ticks_to_wait = (tsc_freq_hz * calibration_time_ms) / 1000;
        let tsc_start = read_tsc();

        loop {
            let tsc_now = read_tsc();
            if tsc_now - tsc_start >= tsc_ticks_to_wait {
                break;
            }
        }

        let apic_end_count = x2apic_read(x2regs::TIMER_CURRENT_COUNT) as u64;
        let apic_ticks_elapsed = 0xFFFFFFFF_u64 - apic_end_count;
        let apic_freq_hz = (apic_ticks_elapsed * 1000) / calibration_time_ms;

        stop_timer();

        if apic_freq_hz == 0 {
            None
        } else {
            Some(apic_freq_hz)
        }
    }
}
