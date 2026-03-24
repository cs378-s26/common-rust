use core::arch::asm;

use spin::Once;

use crate::print::CharSink;
use crate::virtual_memory::PagingOptions;

pub mod apic;
mod asm;
mod context;
mod devices;
mod interrupt;
mod mp;
use alloc::boxed::Box;

pub use apic::timer_ticks;
pub use asm::*;
pub use context::Context;
use context::save_context;
pub use interrupt::*;
use mp::{
    get_cpu_local_pointer, get_thread_local_pointer, init_cpu_local_ptr, initialize_core,
    set_thread_local_pointer,
};
mod vmm;

pub use crate::arch::{ArchTrait, UnwindContextTrait};

pub struct Arch;

impl ArchTrait for Arch {
    type Context = Context;
    fn is_bsp(req: &limine::request::MpRequest, cpu: &limine::mp::Cpu) -> bool {
        let resp = req
            .get_response()
            .expect("Failed to get response from MpRequest.");
        cpu.mpidr == resp.bsp_mpidr()
    }

    unsafe fn initialize_core(cpu: &limine::mp::Cpu) {
        unsafe { initialize_core(cpu) };
    }

    fn set_irq_enabled(enabled: bool) {
        if enabled {
            enable();
        } else {
            disable();
        }
    }

    fn irq_is_enabled() -> bool {
        irq_is_enabled()
    }

    fn sleep_core() {
        asm::sleep_core();
    }

    fn wake_other_cores() {
        apic::send_ipi_all_except_self(IPI_WAKE_VECTOR);
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

    fn shutdown(_err_code: u16) {
        // TODO implement this
        halt();
    }

    fn halt() -> ! {
        halt()
    }

    fn parse_devices() {
        devices::parse_devices();
    }

    fn create_arch_specific_drivers() {
        // create drivers for devices that are specific to this architecture, for example aarch64's uart_pl011
        devices::create_arch_specific_drivers();
    }

    fn init_arch_specific_drivers() {
        devices::init_arch_specific_drivers();
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

pub fn init_tty(_cell: &Once<Box<dyn CharSink>>) {
    // no op for aarch64, serial is implemented via uart_pl011 so devices must be parsed
}
