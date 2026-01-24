use core::arch::asm;
use core::arch::naked_asm;

use x86::bits64::registers::rbp;
use x86::bits64::rflags;
use x86::bits64::rflags::RFlags;
use x86::cpuid::CpuId;
use x86::cpuid::FeatureInfo;

pub fn halt() -> ! {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        asm!("cli");
        loop {
            asm!("hlt");
        }
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

#[inline(always)]
fn disable_interrupts() {
    unsafe {
        asm!("cli");
    }
}

#[inline(always)]
fn enable_interrupts() {
    unsafe {
        asm!("sti");
    }
}

#[repr(transparent)]
#[derive(PartialEq, Eq, Clone, Copy)]
pub struct IrqState(bool);

impl IrqState {
    #[inline(always)]
    pub fn save() -> IrqState {
        IrqState(rflags::read().contains(RFlags::FLAGS_IF))
    }

    #[inline(always)]
    pub fn restore(self) {
        if self.0 {
            enable_interrupts();
        } else {
            disable_interrupts();
        }
    }
}

#[inline(always)]
pub fn irq_disable() {
    disable_interrupts();
}

#[derive(Clone, Copy)]
pub struct UnwindContext {
    ptr: *const u64,
}

impl UnwindContext {
    #[inline(always)]
    pub unsafe fn get() -> UnwindContext {
        UnwindContext {
            ptr: rbp() as *const u64,
        }
    }

    pub unsafe fn valid(&self) -> bool {
        (unsafe { self.return_address() }) != 0
    }

    pub unsafe fn return_address(&self) -> u64 {
        unsafe { self.ptr.wrapping_add(1).read() }
    }

    pub unsafe fn next(&self) -> UnwindContext {
        UnwindContext {
            ptr: unsafe { self.ptr.read() } as *const u64,
        }
    }
}

