use core::arch::asm;

pub fn halt() -> ! {
    unsafe {
        // Mask debug, SError, IRQ, and FIQ before halting.
        asm!(
            "msr daifset, #0xf",
            options(nomem, nostack, preserves_flags)
        );
    }

    loop {
        unsafe {
            asm!("wfi", options(nomem, nostack, preserves_flags));
        }
    }
}

pub fn sleep_core() {
    unsafe {
        asm!("wfi", options(nomem, nostack, preserves_flags));
    }
}
