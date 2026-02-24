use core::cell::SyncUnsafeCell;

use spin::Once;
use uart_16550::SerialPort;
use x86::bits64::registers::rbp;

mod asm;
mod context;
mod cpuid;
mod interrupt;
mod mp;
mod tables;
mod debug;

pub use asm::*;
pub use context::*;
pub use interrupt::*;
pub use mp::*;
pub use debug::*;

use crate::print::CharSink;

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

fn slice_stack_pointer(slice: &[u8]) -> u64 {
    slice.as_ptr_range().end as u64
}

pub struct SerialCharSink {
    serial: SyncUnsafeCell<SerialPort>,
}

impl SerialCharSink {
    pub fn open(port: u16) -> SerialCharSink {
        let mut serial = unsafe { SerialPort::new(port) };
        serial.init();
        SerialCharSink {
            serial: SyncUnsafeCell::new(serial),
        }
    }
}

impl CharSink for SerialCharSink {
    unsafe fn putc(&self, ch: u8) {
        unsafe { &mut *self.serial.get() }.send(ch);
    }

    unsafe fn flush(&self) {
        // no-op
    }
}

pub fn init_tty(cell: &Once<SerialCharSink>) {
    cell.call_once(|| SerialCharSink::open(0x3f8));
}
