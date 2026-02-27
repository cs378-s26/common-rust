use core::arch::asm;
use core::arch::naked_asm;

pub fn read_cycle_counter() -> u64 {
    let lo: u32;
    let hi: u32;

    unsafe {
        asm!(
            "rdtsc",
            out("eax") lo,
            out("edx") hi,
            options(nomem, nostack, preserves_flags),
        );
    }

    ((hi as u64) << 32) | (lo as u64)
}

pub fn halt() -> ! {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        asm!("cli");
        loop {
            asm!("hlt");
        }
    }
}

pub fn sleep_core() {
    unsafe {
        asm!("sti", "hlt", options(nomem, nostack));
    }
}

#[unsafe(naked)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcpy(dest: *mut u8, src: *const u8, len: usize) -> *mut u8 {
    naked_asm!(
        "mov rax, rdi",
        "mov rcx, rdx",
        "shr rcx, 3",
        "rep movsq",
        "mov rcx, rdx",
        "and rcx, 0x7",
        "rep movsb",
        "ret",
    )
}

#[unsafe(naked)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memset(dest: *mut u8, byte: i32, len: usize) -> *mut u8 {
    naked_asm!(
        "mov r11, rdi",
        "mov rcx, rdx",
        "movzx rax, sil",
        "mov r10, 0x0101010101010101",
        "mul r10",
        "mov rdx, rcx",
        "shr rcx, 3",
        "rep stosq",
        "mov rcx, rdx",
        "and rcx, 0x7",
        "rep stosb",
        "mov rax, r11",
        "ret",
    )
}
