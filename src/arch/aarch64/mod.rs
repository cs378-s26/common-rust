use core::arch::asm;

use spin::Once;

use crate::arch::IrqStateTrait;
use crate::print::CharSink;
use crate::virtual_memory::PagingOptions;

pub mod apic;
mod asm;
mod context;
mod exceptions;
pub mod gic;
mod interrupt;
mod mp;
pub use exceptions::{dump_core_state, init_exceptions};
mod vmm;

pub use asm::*;
pub use context::Context;
use context::save_context;
pub use gic::timer_ticks;
pub use interrupt::*;
use mp::{
    get_cpu_local_pointer, get_thread_local_pointer, init_cpu_local_ptr, initialize_core,
    set_thread_local_pointer,
};

pub use crate::arch::{ArchTrait, UnwindContextTrait};

pub struct Arch;

impl ArchTrait for Arch {
    type Context = Context;
    type IrqState = IrqState;
    fn is_bsp(req: &limine::request::MpRequest, cpu: &limine::mp::Cpu) -> bool {
        let resp = req
            .get_response()
            .expect("Failed to get response from MpRequest.");
        cpu.mpidr == resp.bsp_mpidr()
    }

    unsafe fn initialize_core(cpu: &limine::mp::Cpu) -> () {
        unsafe { initialize_core(cpu) };
    }

    fn set_irq_enabled(enabled: bool) {
        unsafe {
            if enabled {
                enable();
            } else {
                disable();
            }
        }
    }

    fn irq_is_enabled() -> bool {
        !IrqState::save().is_masked()
    }

    unsafe fn save_context<T: FnOnce() -> !>(
        temp_stack: &[u8],
        ctx: spin::MutexGuard<'static, Self::Context>,
        fwd: T,
    ) {
        unsafe {
            save_context(temp_stack, ctx, fwd);
        }
    }

    fn get_cpu_local_pointer() -> u64 {
        get_cpu_local_pointer()
    }

    fn set_cpu_local_pointer(core_id: crate::mp::CoreId) {
        init_cpu_local_ptr(core_id);
    }

    unsafe fn get_thread_local_pointer() -> u64 {
        unsafe { get_thread_local_pointer() }
    }

    unsafe fn set_thread_local_pointer(base: *const u64) {
        unsafe { set_thread_local_pointer(base) };
    }

    fn read_cycle_counter() -> u64 {
        asm::read_cycle_counter()
    }

    fn timer_ticks() -> u64 {
        gic::timer_ticks()
    }

    const PAGE_SIZE: usize = 4096;

    fn get_address_space() -> u64 {
        vmm::get_address_space()
    }

    fn virtual_map(space: u64, vaddr: u64, paddr: u64, options: PagingOptions) {
        vmm::vmap(space, vaddr, paddr, options)
    }

    fn virtual_unmap(_space: u64, _vaddr: u64) -> Option<u64> {
        vmm::vunmap(_space, _vaddr)
    }

    fn vaddr_to_paddr(space: u64, vaddr: u64) -> Option<u64> {
        vmm::vaddr_to_paddr(space, vaddr)
    }

    fn shutdown(_err_code: u16) {
        // TODO implement this
        halt();
    }

    fn halt() -> ! {
        halt()
    }
}

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
