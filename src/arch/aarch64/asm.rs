use core::arch::asm;

pub fn read_cycle_counter() -> u64 {
    let value: u64;

    unsafe {
        asm!(
            "mrs {}, cntvct_el0",
            out(reg) value,
            options(nomem, nostack, preserves_flags)
        );
    }

    value
}

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

pub unsafe extern "C" fn switch_stack(_stack_top: u64, _f: extern "C" fn() -> !) -> ! {
    panic!("switch_stack not implemented on aarch64");
}
