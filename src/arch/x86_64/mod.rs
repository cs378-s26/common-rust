use crate::devices::device_discovery::DeviceDiscovery;
use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::{
    cell::SyncUnsafeCell,
    hint,
    sync::atomic::{AtomicUsize, Ordering},
};

use limine::{mp::Cpu, request::MpRequest};
use spin::Once;
use uart_16550::SerialPort;
use x86::bits64::registers::rbp;

pub mod apic;
mod asm;
mod context;
mod cpuid;
mod debug;
mod interrupt;
mod mp;
mod tables;
pub mod tsc;
mod vmm;

pub use asm::*;
pub use context::Context;
use context::save_context;
pub use interrupt::*;
pub use irq_vector::IPI_WAKE;
use mp::{
    get_cpu_local_pointer, get_thread_local_pointer, init_cpu_local_ptr, initialize_core,
    set_thread_local_pointer,
};
pub use vmm::*;
use x86::{
    bits64::rflags::{self, RFlags},
    irq,
    tlb::flush,
};

use crate::{
    MP_REQUEST,
    arch::{
        ArchTrait, UnwindContextTrait, apic::send_ipi_all_except_self, irq_vector::TLB_SHOOTDOWN,
    },
    event::{Event::Shootdown, push_event},
    mp::{CORE_ID, CoreId},
    print::CharSink,
    thread::yield_thread,
    virtual_memory::PagingOptions,
};
pub struct Arch;

impl ArchTrait for Arch {
    type Context = Context;

    fn page_size() -> usize {
        Self::PAGE_SIZE
    }

    fn is_bsp(req: &MpRequest, cpu: &Cpu) -> bool {
        let resp = req
            .get_response()
            .expect("Failed to get response from MpRequest.");
        cpu.lapic_id == resp.bsp_lapic_id()
    }

    unsafe fn initialize_core(cpu: &Cpu) {
        unsafe { initialize_core(cpu) };
    }

    fn set_irq_enabled(enabled: bool) {
        unsafe {
            if enabled {
                irq::enable();
            } else {
                irq::disable();
            }
        }
    }

    fn irq_is_enabled() -> bool {
        rflags::read().contains(RFlags::FLAGS_IF)
    }

    fn sleep_core() {
        asm::sleep_core();
    }

    fn wake_other_cores() {
        apic::send_ipi_all_except_self(IPI_WAKE);
    }

    unsafe fn save_context<T: FnOnce() -> !>(
        temp_stack: &[u8],
        ctx: spin::MutexGuard<'static, Self::Context>,
        fwd: T,
    ) {
        unsafe { save_context(temp_stack, ctx, fwd) };
    }

    fn get_cpu_local_pointer() -> u64 {
        get_cpu_local_pointer()
    }

    fn set_cpu_local_pointer(core_id: CoreId) {
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

    fn configure_vm() {}

    fn get_address_space() -> u64 {
        get_address_space()
    }

    fn virtual_map(space: u64, vaddr: u64, paddr: u64, options: PagingOptions) {
        vmap(space, vaddr, paddr, options);
    }

    fn virtual_unmap(space: u64, vaddr: u64) -> Option<u64> {
        vunmap(space, vaddr)
    }

    fn virtual_invalidate(vaddr: u64) {
        unsafe { flush(vaddr as usize) };
    }

    fn shootdown_tlbs(space: u64, base: usize, length: usize) {
        let num_cores = MP_REQUEST.get_response().unwrap().cpus().len(); // TODO replace with global variable
        let latch = Arc::new(AtomicUsize::new(num_cores - 1)); // there had better be at least one lol
        let me = CORE_ID.get();
        for core in 0..num_cores {
            if core != me.0 {
                // vunmap already handles this core
                push_event(
                    Shootdown {
                        space,
                        base,
                        length,
                        latch: latch.clone(),
                    },
                    CoreId(core),
                ); // TODO avoid sending this when not needed
            }
        }

        send_ipi_all_except_self(TLB_SHOOTDOWN);
        while latch.load(Ordering::Acquire) != 0 {
            yield_thread(); // TODO block on this
            hint::spin_loop();
        }
    }

    fn shutdown(err_code: u16) {
        debug::shutdown(err_code);
    }

    fn halt() -> ! {
        halt()
    }

    fn parse_devices() {}
    fn create_arch_specific_drivers(
        _system_drivers: &mut Vec<Box<dyn DeviceDiscovery + Send + Sync>>,
    ) {
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
        UnwindContext {
            ptr: rbp() as *const u64,
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

pub fn init_tty(cell: &Once<Box<dyn CharSink>>) {
    cell.call_once(|| Box::new(SerialCharSink::open(0x3f8)));
}
