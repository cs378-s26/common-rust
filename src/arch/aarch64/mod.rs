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

pub use crate::arch::{ArchTrait, ContextTrait, IrqStateTrait, UnwindContextTrait};

pub struct Aarch64;

#[derive(Clone, Copy)]
pub struct UnwindContext {
    ptr: *const u64,
}

impl UnwindContextTrait for UnwindContext {
    fn from_ptr(ptr: *const u64) -> UnwindContext {
        UnwindContext { ptr }
    }
    fn get_ptr(&self) -> *const u64 {
        self.ptr
    }
    #[inline(always)]
    unsafe fn get() -> UnwindContext {
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
