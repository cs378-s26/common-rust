#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_common::test_runner)]

kernel_common::integration_test!({
    extern crate alloc;
    use alloc::boxed::Box;
    use core::sync::atomic::{AtomicBool, Ordering};

    use kernel_common::{
        arch::{Arch, ArchTrait, apic, timer_ticks},
        print::kprintln,
    };

    static FIRED: AtomicBool = AtomicBool::new(false);

    kprintln!("Testing IRQ handler registration...");

    Arch::register_irq_handler(
        8,
        Box::new(|| {
            unsafe {
                use x86::io::{inb, outb};
                // Reading register C re-arms the RTC and reveals why it fired.
                // Bit 6 (PF) is the Periodic Interrupt Flag; only count that.
                outb(0x70, 0x0C);
                let reg_c = inb(0x71);
                if reg_c & 0x40 != 0 && !FIRED.load(Ordering::Relaxed) {
                    FIRED.store(true, Ordering::Relaxed);
                    kprintln!("Irq handler called!");
                }
            }
            apic::eoi();
            Some(())
        }),
    );

    // Enable RTC periodic interrupts via CMOS register B (IRQ 8).
    // The default rate is ~1024 Hz, so the interrupt fires almost immediately.
    unsafe {
        use x86::io::{inb, outb};
        outb(0x70, 0x8B); // select register B, NMI disabled
        let prev = inb(0x71);
        outb(0x70, 0x8B); // re-select (CMOS index resets after each read/write)
        outb(0x71, prev | 0x40); // set PIE (periodic interrupt enable)
        outb(0x70, 0x8C); // read register C to clear any stale pending interrupt
        let _ = inb(0x71);
    }

    // Wait up to ~several seconds (based on APIC timer ticks) for the RTC to fire.
    let deadline = timer_ticks() + 2000;
    while !FIRED.load(Ordering::Relaxed) && timer_ticks() < deadline {
        core::hint::spin_loop();
    }

    if FIRED.load(Ordering::Relaxed) {
        kprintln!("IRQ handler fired!");
    } else {
        kprintln!("IRQ handler did not fire.");
    }
});
