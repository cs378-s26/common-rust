use core::arch::asm;

pub struct IoPort;

impl IoPort {
    #[inline(always)]
    pub fn inb(port: u16) -> u8 {
        let value: u8;
        unsafe { asm!("in al, dx", out("al") value, in("dx") port,
            options(nomem, nostack, preserves_flags)) };
        value
    }

    #[inline(always)]
    pub fn inw(port: u16) -> u16 {
        let value: u16;
        unsafe { asm!("in ax, dx", out("ax") value, in("dx") port,
            options(nomem, nostack, preserves_flags)) };
        value
    }

    #[inline(always)]
    pub fn inl(port: u16) -> u32 {
        let value: u32;
        unsafe { asm!("in eax, dx", out("eax") value, in("dx") port,
            options(nomem, nostack, preserves_flags)) };
        value
    }

    #[inline(always)]
    pub fn outb(port: u16, value: u8) {
        unsafe { asm!("out dx, al", in("al") value, in("dx") port,
            options(nomem, nostack, preserves_flags)) };
    }

    #[inline(always)]
    pub fn outw(port: u16, value: u16) {
        unsafe { asm!("out dx, ax", in("ax") value, in("dx") port,
            options(nomem, nostack, preserves_flags)) };
    }

    #[inline(always)]
    pub fn outl(port: u16, value: u32) {
        unsafe { asm!("out dx, eax", in("eax") value, in("dx") port,
            options(nomem, nostack, preserves_flags)) };
    }
}
