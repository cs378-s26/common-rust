use core::arch::asm;

use spin::Once;

use crate::print::CharSink;
use crate::memory::virtual_memory::PagingOptions;

use crate::devices::discovery::DeviceDiscovery;
use alloc::boxed::Box;
use alloc::vec::Vec;

mod asm;
mod context;
mod devices;
mod exceptions;
pub mod gic;
mod interrupt;
mod mp;
pub use exceptions::{dump_core_state, init_exceptions};

pub use asm::*;
pub use context::Context;
use context::save_context;
pub use gic::timer_ticks;
pub use interrupt::*;
use mp::{
    get_cpu_local_pointer, get_thread_local_pointer, init_cpu_local_ptr, initialize_core,
    set_thread_local_pointer,
};
mod vmm;

pub use crate::arch::{ArchTrait, UnwindContextTrait};

pub struct Arch;

use devices::psci::PSCI_DEVICE;

impl ArchTrait for Arch {
    type Context = Context;

    fn page_size() -> usize {
        Self::PAGE_SIZE
    }
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

    // TODO implement this
    // doesn't really affect correctness just can give a performance boost
    fn wake_other_cores() {}

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

    fn get_kernel_address_space() -> u64 {
        vmm::get_kernel_address_space()
    }

    fn get_user_address_space() -> u64 {
        vmm::get_user_address_space()
    }

    fn set_user_address_space(space: u64) {
        vmm::set_user_address_space(space)
    }

    fn virtual_map(space: u64, vaddr: u64, paddr: u64, options: PagingOptions) {
        vmm::vmap(space, vaddr, paddr, options)
    }

    fn virtual_unmap(space: u64, vaddr: u64) -> Option<u64> {
        vmm::vunmap(space, vaddr)
    }

    // no-op on aarch64
    fn virtual_invalidate(_vaddr: u64) {}

    // TODO this needs to be made more flexible to allow different kinds of shootdowns, not just global
    fn shootdown_tlbs(_space: u64, base: usize, length: usize) {
        for page in (0..length).step_by(Self::PAGE_SIZE) {
            vmm::tlb_shootdown((base + page) as u64);
        }
    }

    fn virtual_unmap_no_dealloc(_space: u64, _vaddr: u64) -> Option<u64> {
        vmm::vunmap_no_dealloc(_space, _vaddr)
    }

    fn shutdown(_err_code: u16) {
        PSCI_DEVICE
            .get()
            .expect("PSCI device not found, cannot shutdown") // very critical this is set, otherwise you get in an infinite shutdown loop
            .shutdown();
    }

    fn halt() -> ! {
        halt()
    }

    fn configure_vm() {
        vmm::configure_vm();
    }

    fn create_arch_specific_drivers(
        system_drivers: &mut Vec<Box<dyn DeviceDiscovery + Send + Sync>>,
    ) {
        // create drivers for devices that are specific to this architecture, for example aarch64's uart_pl011
        devices::create_arch_specific_drivers(system_drivers);
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
