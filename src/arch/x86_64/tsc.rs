use core::arch::asm;

use x86::io::{inb, outb};

#[inline]
pub fn read_tsc() -> u64 {
    let low: u32;
    let high: u32;
    unsafe {
        asm!(
            "rdtsc",
            out("eax") low,
            out("edx") high,
            options(nomem, nostack, preserves_flags)
        );
    }
    ((high as u64) << 32) | (low as u64)
}

pub fn calibrate_tsc_with_pit() -> u64 {
    const PIT_FREQUENCY: u64 = 1193182;
    const PIT_CHANNEL_0: u16 = 0x40;
    const PIT_COMMAND: u16 = 0x43;
    const PIT_DIVISOR: u16 = 65535;

    let calibration_time_us = (PIT_DIVISOR as u64 * 1_000_000) / PIT_FREQUENCY;

    unsafe {
        asm!("cli");

        outb(PIT_COMMAND, 0b00110000);
        outb(PIT_CHANNEL_0, (PIT_DIVISOR & 0xFF) as u8);
        outb(PIT_CHANNEL_0, ((PIT_DIVISOR >> 8) & 0xFF) as u8);

        let tsc_start = read_tsc();

        loop {
            outb(PIT_COMMAND, 0b00000000);
            let low = inb(PIT_CHANNEL_0) as u16;
            let high = inb(PIT_CHANNEL_0) as u16;
            let count = (high << 8) | low;

            if count == 0 {
                break;
            }
        }

        let tsc_end = read_tsc();

        asm!("sti");

        let tsc_delta = tsc_end - tsc_start;
        (tsc_delta * 1_000_000) / calibration_time_us
    }
}
