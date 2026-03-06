use core::arch::asm;

use spin::Once;

use crate::print::CharSink;
use crate::virtual_memory::PagingOptions;

pub mod apic;
mod asm;
mod context;
mod device_tree;
use device_tree::DeviceRegistry;
mod interrupt;
mod mp;

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

// populated once at boot by init_device_discovery; readable by the rest of the kernel
static DEVICE_REGISTRY: Once<DeviceRegistry> = Once::new();

pub struct Arch;

impl ArchTrait for Arch {
    type Context = Context;
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
        irq_is_enabled()
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

    fn init_device_discovery() {
        if let Some(dtb) = crate::DEVICE_TREE_BLOB_REQUEST.get_response() {
            // safety: limine guarantees this pointer is valid for the lifetime of the kernel
            let registry = unsafe { device_tree::walk_device_tree(dtb.dtb_ptr()) };

            for dev in &registry.devices {
                let compat = dev.compatible.first().copied().unwrap_or("(none)");
                match dev.mmio {
                    Some((base, size)) => crate::print::kprintln!(
                        "  device: {}  compatible: {}  mmio: {:#x} size: {:#x}",
                        dev.name, compat, base, size
                    ),
                    None => crate::print::kprintln!(
                        "  device: {}  compatible: {}  mmio: none",
                        dev.name, compat
                    ),
                }
            }

            // store registry so other parts of the kernel can query it later
            DEVICE_REGISTRY.call_once(|| registry);
        } else {
            crate::print::kprintln!("device_tree: no dtb from bootloader");
        }
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
