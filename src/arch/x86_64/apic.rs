use crate::mp::CORE_ID;
use crate::{mp::core_local, physical_memory::HHDM_REQUEST, virtual_memory::PagingOptions};
use spin::Once;
use x86::cpuid::CpuId;
use x86::io::outb;
use x86::msr::{rdmsr, wrmsr};

use super::tsc::read_tsc;
use super::{get_address_space, vmap};

#[derive(Clone, Copy, PartialEq, Eq)]
enum ApicState {
    XApic,
    X2Apic,
}

core_local! {
    APIC_STATE: Once<ApicState> = Once::new();
}

static APIC_MMIO_BASE: Once<u64> = Once::new();

const X2APIC_MSR_BASE: u32 = 0x800;

mod x2regs {
    pub const ID: u32 = 0x02;
    pub const TPR: u32 = 0x08;
    pub const EOI: u32 = 0x0B;
    pub const SIVR: u32 = 0x0F;
    pub const ESR: u32 = 0x28;
    pub const ICR: u32 = 0x30;
    pub const TIMER_LVT: u32 = 0x32;
    pub const TIMER_INITIAL_COUNT: u32 = 0x38;
    pub const TIMER_CURRENT_COUNT: u32 = 0x39;
    pub const TIMER_DIVIDE_CONFIG: u32 = 0x3E;
    pub const LINT0_LVT: u32 = 0x35;
    pub const LINT1_LVT: u32 = 0x36;
    pub const ERROR_LVT: u32 = 0x37;
}

mod xregs {
    pub const ID: u32 = 0x20;
    pub const EOI: u32 = 0xB0;
    pub const SIVR: u32 = 0xF0;
    pub const ICR_LOW: u32 = 0x300;
    pub const ICR_HIGH: u32 = 0x310;
    pub const TIMER_LVT: u32 = 0x320;
    pub const TIMER_INITIAL_COUNT: u32 = 0x380;
    pub const TIMER_CURRENT_COUNT: u32 = 0x390;
    pub const TIMER_DIVIDE_CONFIG: u32 = 0x3E0;
}

const IA32_APIC_BASE_MSR: u32 = 0x1B;
const APIC_BASE_ENABLE: u64 = 1 << 11;
const APIC_BASE_X2APIC: u64 = 1 << 10;
const SIVR_APIC_ENABLED: u64 = 0x100;
const SPURIOUS_VECTOR: u64 = 0xFF;

#[inline]
unsafe fn x2apic_read(reg: u32) -> u64 {
    unsafe { rdmsr(X2APIC_MSR_BASE + reg) }
}

#[inline]
unsafe fn x2apic_write(reg: u32, value: u64) {
    unsafe { wrmsr(X2APIC_MSR_BASE + reg, value) };
}

#[inline]
fn xapic_read(reg: u32) -> u32 {
    let hhdm_base = HHDM_REQUEST.get_response().unwrap().offset();
    unsafe {
        core::ptr::read_volatile(
            (APIC_MMIO_BASE.get().unwrap() + hhdm_base + (reg as u64)) as *const u32,
        )
    }
}

#[inline]
fn xapic_write(reg: u32, value: u32) {
    let hhdm_base = HHDM_REQUEST.get_response().unwrap().offset();
    unsafe {
        core::ptr::write_volatile(
            (APIC_MMIO_BASE.get().unwrap() + hhdm_base + (reg as u64)) as *mut u32,
            value,
        )
    };
}

#[inline]
fn xapic_wait_for_ipi_delivery() {
    while (xapic_read(xregs::ICR_LOW) & (1 << 12)) != 0 {
        core::hint::spin_loop();
    }
}

fn map_xapic_mmio(base: u64) -> u64 {
    let hhdm_base = HHDM_REQUEST.get_response().unwrap().offset();
    vmap(
        get_address_space(),
        hhdm_base + base,
        base,
        PagingOptions::PRESENT | PagingOptions::WRITABLE | PagingOptions::GLOBAL,
    );
    base
}

#[inline]
fn apic_state() -> ApicState {
    *APIC_STATE
        .get()
        .unwrap_or_else(|| panic!("APIC not enabled on core {:x}", CORE_ID.get().0))
}

pub fn enable_apic() -> bool {
    let cpu_id = CpuId::new();
    let feature_info = cpu_id.get_feature_info();
    let (state, base_bits) = match feature_info {
        Some(info) if info.has_x2apic() => (ApicState::X2Apic, APIC_BASE_ENABLE | APIC_BASE_X2APIC),
        Some(info) if info.has_apic() => (ApicState::XApic, APIC_BASE_ENABLE),
        _ => return false,
    };

    let mut base = unsafe { rdmsr(IA32_APIC_BASE_MSR) };
    base |= base_bits;

    match state {
        ApicState::XApic => {
            APIC_MMIO_BASE.call_once(|| map_xapic_mmio(base & !0xfff));
        }
        ApicState::X2Apic => {}
    }

    unsafe { wrmsr(IA32_APIC_BASE_MSR, base) };
    APIC_STATE.call_once(|| state);

    true
}

pub fn get_lapic_id() -> u32 {
    match apic_state() {
        ApicState::XApic => xapic_read(xregs::ID) >> 24,
        ApicState::X2Apic => unsafe { x2apic_read(x2regs::ID) as u32 },
    }
}

pub fn init_lapic() {
    match apic_state() {
        ApicState::XApic => {
            // much of the interrupt setup is done by the firmware in xAPIC mode,
            // so we can do much less setup than in x2APIC mode.
            xapic_write(xregs::SIVR, (SIVR_APIC_ENABLED | SPURIOUS_VECTOR) as u32);
        }
        ApicState::X2Apic => unsafe {
            x2apic_write(x2regs::SIVR, SIVR_APIC_ENABLED | SPURIOUS_VECTOR);
            x2apic_write(x2regs::TPR, 0);
            x2apic_write(x2regs::ESR, 0);
            x2apic_write(x2regs::TIMER_LVT, 0x10000);
            x2apic_write(x2regs::LINT0_LVT, 0x10000);
            x2apic_write(x2regs::LINT1_LVT, 0x10000);
            x2apic_write(x2regs::ERROR_LVT, 0x10000);
        },
    }
}

pub fn eoi() {
    match apic_state() {
        ApicState::XApic => xapic_write(xregs::EOI, 0),
        ApicState::X2Apic => unsafe {
            x2apic_write(x2regs::EOI, 0);
        },
    }
}

pub fn disable_pic() {
    unsafe {
        outb(0x21, 0xFF);
        outb(0xA1, 0xFF);
    }
}

pub fn setup_timer(vector: u8, initial_count: u32, periodic: bool) {
    match apic_state() {
        ApicState::XApic => {
            xapic_write(xregs::TIMER_DIVIDE_CONFIG, 0x0B);
            let mut lvt = vector as u32;
            if periodic {
                lvt |= 0x20000;
            }
            xapic_write(xregs::TIMER_LVT, lvt);
            xapic_write(xregs::TIMER_INITIAL_COUNT, initial_count);
        }
        ApicState::X2Apic => unsafe {
            x2apic_write(x2regs::TIMER_DIVIDE_CONFIG, 0x0B);

            let mut lvt = vector as u64;
            if periodic {
                lvt |= 0x20000;
            }
            x2apic_write(x2regs::TIMER_LVT, lvt);
            x2apic_write(x2regs::TIMER_INITIAL_COUNT, initial_count as u64);
        },
    }
}

pub fn stop_timer() {
    match apic_state() {
        ApicState::XApic => {
            xapic_write(xregs::TIMER_INITIAL_COUNT, 0);
            xapic_write(xregs::TIMER_LVT, 0x10000);
        }
        ApicState::X2Apic => unsafe {
            x2apic_write(x2regs::TIMER_INITIAL_COUNT, 0);
            x2apic_write(x2regs::TIMER_LVT, 0x10000);
        },
    }
}

pub fn read_timer() -> u32 {
    match apic_state() {
        ApicState::XApic => xapic_read(xregs::TIMER_CURRENT_COUNT),
        ApicState::X2Apic => unsafe { x2apic_read(x2regs::TIMER_CURRENT_COUNT) as u32 },
    }
}

pub fn calibrate_apic_timer_with_tsc(tsc_freq_hz: u64) -> Option<u64> {
    stop_timer();

    match apic_state() {
        ApicState::XApic => {
            xapic_write(xregs::TIMER_DIVIDE_CONFIG, 0x0B);
            xapic_write(xregs::TIMER_LVT, 0x10000 | SPURIOUS_VECTOR as u32);
            xapic_write(xregs::TIMER_INITIAL_COUNT, u32::MAX);
        }
        ApicState::X2Apic => unsafe {
            x2apic_write(x2regs::TIMER_DIVIDE_CONFIG, 0x0B);
            x2apic_write(x2regs::TIMER_LVT, 0x10000 | SPURIOUS_VECTOR);
            x2apic_write(x2regs::TIMER_INITIAL_COUNT, u32::MAX as u64);
        },
    }

    let calibration_time_ms = 10;
    let tsc_ticks_to_wait = (tsc_freq_hz * calibration_time_ms) / 1000;
    let tsc_start = read_tsc();

    loop {
        let tsc_now = read_tsc();
        if tsc_now - tsc_start >= tsc_ticks_to_wait {
            break;
        }
    }

    let apic_end_count = read_timer() as u64;
    let apic_ticks_elapsed = u32::MAX as u64 - apic_end_count;
    let apic_freq_hz = (apic_ticks_elapsed * 1000) / calibration_time_ms;

    stop_timer();

    if apic_freq_hz == 0 {
        None
    } else {
        Some(apic_freq_hz)
    }
}

pub fn send_ipi(dest_apic_id: u32, vector: u8) {
    match apic_state() {
        ApicState::XApic => {
            xapic_wait_for_ipi_delivery();
            xapic_write(xregs::ICR_HIGH, dest_apic_id << 24);
            xapic_write(xregs::ICR_LOW, (1 << 14) | vector as u32);
            xapic_wait_for_ipi_delivery();
        }
        ApicState::X2Apic => {
            let icr = ((dest_apic_id as u64) << 32) | (1 << 14) | vector as u64;
            unsafe { x2apic_write(x2regs::ICR, icr) };
        }
    }
}

pub fn send_ipi_all_except_self(vector: u8) {
    match apic_state() {
        ApicState::XApic => {
            xapic_wait_for_ipi_delivery();
            xapic_write(xregs::ICR_HIGH, 0);
            xapic_write(xregs::ICR_LOW, (0b11 << 18) | (1 << 14) | vector as u32);
            xapic_wait_for_ipi_delivery();
        }
        ApicState::X2Apic => {
            let icr = (0b11 << 18) | (1 << 14) | vector as u64;
            unsafe { x2apic_write(x2regs::ICR, icr) };
        }
    }
}
