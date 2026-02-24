use core::arch::asm;

use spin::Once;

use crate::print::CharSink;

mod asm;
mod context;
mod interrupt;
mod mp;

pub use asm::*;
pub use context::*;
pub use interrupt::*;
pub use mp::*;

// use crate::arch::Arch;

// pub struct Aarch64;

// impl Arch for Aarch64 {
//     fn initialize_mp(req: limine::request::MpRequest) {
//         mp::initialize_mp(req);
//     }

//     unsafe fn initialize_core() {
//         mp::initialize_core();
//         interrupt::initialize_interrupts();
//     }
// }

#[derive(Clone, Copy)]
pub struct UnwindContext {
    ptr: *const u64,
}

impl UnwindContext {
    #[inline(always)]
    pub unsafe fn get() -> UnwindContext {
        let fp: u64;
        unsafe {
            asm!(
                "mov {}, x29",
                out(reg) fp,
                options(nomem, nostack, preserves_flags)
            );
        }

        UnwindContext {
            ptr: fp as *const u64,
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

pub struct SerialCharSink;

impl SerialCharSink {
    pub fn open(_port: u16) -> SerialCharSink {
        SerialCharSink
    }
}

impl CharSink for SerialCharSink {
    unsafe fn putc(&self, _ch: u8) {}

    unsafe fn flush(&self) {}
}

pub fn init_tty(cell: &Once<SerialCharSink>) {
    cell.call_once(|| SerialCharSink::open(0));
}
