use x86::{
    fence::lfence,
    io::{inb, outb},
    time::rdtsc,
};

#[inline]
pub fn read_tsc() -> u64 {
    unsafe {
        lfence();
        rdtsc()
    }
}

fn pit_wait_ms(n_ms: u16) {
    const PIT_CHANNEL_0: u16 = 0x40;
    const PIT_COMMAND: u16 = 0x43;
    const TIMER_INIT_VALUE: u16 = 0xffff;

    fn write_pit_timer(ticks: u16) {
        unsafe {
            outb(PIT_COMMAND, 0b00110000);
            outb(PIT_CHANNEL_0, ticks.to_le_bytes()[0]);
            outb(PIT_CHANNEL_0, ticks.to_le_bytes()[1]);
        }
    }

    fn read_pit_timer() -> u16 {
        unsafe {
            outb(PIT_COMMAND, 0);
            u16::from_le_bytes([inb(PIT_CHANNEL_0), inb(PIT_CHANNEL_0)])
        }
    }

    write_pit_timer(0xFFFF);

    loop {
        let count = read_pit_timer();

        if TIMER_INIT_VALUE - count > 1193 * n_ms {
            break;
        }
    }
}

pub fn calibrate_tsc_with_pit() -> u64 {
    let tsc_start = read_tsc();
    pit_wait_ms(10);
    let tsc_end = read_tsc();
    let tsc_delta = tsc_end - tsc_start;

    tsc_delta * 100
}
