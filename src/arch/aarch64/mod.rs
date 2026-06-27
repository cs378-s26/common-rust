use alloc::{boxed::Box, vec::Vec};
use core::{arch::asm, ptr};

use spin::Once;

use crate::{
    arch::aarch64::vmm::{get_phys_addr, phys_to_virt},
    devices::discovery::DeviceDiscovery,
    memory::{physical_memory::frame_alloc, virtual_memory::PagingOptions},
    print::CharSink,
};

mod asm;
mod context;
mod devices;
mod exceptions;
pub mod gic;
mod interrupt;
mod mp;
pub use asm::*;
pub use context::Context;
use context::save_context;
pub use exceptions::{dump_core_state, init_exceptions};
pub use gic::timer_ticks;
pub use interrupt::*;
use mp::{get_cpu_local_pointer, init_cpu_local_ptr, initialize_core};
mod vmm;

pub use crate::arch::{ArchTrait, UnwindContextTrait};

const AARCH64_STACK_ALIGNMENT: u64 = 64;

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

    // TODO implement proper auxv handling and envp
    fn setup_stack(sp: u64, space: u64, argc: u64, argv: &[&str], envp: &[&str]) -> Option<u64> {
        let mut sp = sp;
        assert!(argv.len() as u64 == argc);
        assert!(
            sp % 16 == 0,
            "Stack pointer must be 16-byte aligned on aarch64"
        );
        // write using the kernel virtual address to not deal with user space mappings

        let mut arg_ptrs = Vec::new();
        let mut env_ptrs = Vec::new();

        // push the arguments onto the stack in reverse order
        for arg in argv.iter() {
            let bytes = arg.as_bytes();
            sp -= (bytes.len() + 1) as u64; // +1 for null terminator
            copy_to_user(space, sp, bytes).ok()?;
            copy_to_user(space, sp + bytes.len() as u64, &[0]).ok()?; // null terminator
            arg_ptrs.push(sp as u64);
        }

        for env in envp.iter() {
            let bytes = env.as_bytes();
            sp -= (bytes.len() + 1) as u64; // +1 for null terminator
            copy_to_user(space, sp, bytes).ok()?;
            copy_to_user(space, sp + bytes.len() as u64, &[0]).ok()?; // null terminator
            env_ptrs.push(sp as u64);
        }

        let num_words = arg_ptrs.len() + env_ptrs.len() + 5; // 5 for the two null terminators for envp and argv, two words for auxv, and argc
        sp -= (num_words * 8) as u64; // make space for the pointers and auxv
        sp &= !(AARCH64_STACK_ALIGNMENT - 1);
        let mut temp_sp = sp;

        // this should technically be replaced by a copy_u64 for speed, but this is fine for now
        copy_to_user(space, temp_sp, &argc.to_ne_bytes()).ok()?; // argc
        temp_sp += 8;

        for ptr in arg_ptrs.iter() {
            copy_to_user(space, temp_sp, &ptr.to_ne_bytes()).ok()?;
            temp_sp += 8;
        }
        copy_to_user(space, temp_sp, &[0; 8]).ok()?; // NULL terminator for argv
        temp_sp += 8;
        for ptr in env_ptrs.iter() {
            copy_to_user(space, temp_sp, &ptr.to_ne_bytes()).ok()?;
            temp_sp += 8;
        }
        copy_to_user(space, temp_sp, &[0; 8]).ok()?; // NULL terminator for envp
        temp_sp += 8;
        copy_to_user(space, temp_sp, &[0; 16]).ok()?; // NULL terminator for auxv
        return Some(sp as u64);
    }

    fn sleep_core() {
        asm::sleep_core();
    }

    // TODO implement this
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

    fn init_tty(_cell: &Once<Box<dyn CharSink>>) {
        // no op for aarch64, serial is implemented via uart_pl011 so devices must be parsed
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

fn copy_to_user(space: u64, mut dst: u64, mut bytes: &[u8]) -> Result<(), ()> {
    while !bytes.is_empty() {
        ensure_user_page(space, dst)?;
        let kva = phys_to_virt(get_phys_addr(dst, space).ok_or(())?);
        let page_left = Arch::PAGE_SIZE - (dst as usize % Arch::PAGE_SIZE);
        let bytes_left = bytes.len();
        let to_copy = core::cmp::min(page_left, bytes_left);
        unsafe {
            ptr::copy_nonoverlapping(bytes.as_ptr(), kva as usize as *mut u8, to_copy);
        }
        dst += to_copy as u64;
        bytes = &bytes[to_copy..];
    }
    Ok(())
}

// TODO we'll need some pinning system so after we ensure a page is present it doesn't get swapped out, but this will come with swap implementation
fn ensure_user_page(space: u64, vaddr: u64) -> Result<(), ()> {
    if get_phys_addr(vaddr, space).is_some() {
        return Ok(());
    }

    let frame = frame_alloc();
    Arch::virtual_map(
        space,
        vaddr & !(Arch::PAGE_SIZE as u64 - 1),
        frame as u64,
        PagingOptions::PRESENT
            | PagingOptions::WRITABLE
            | PagingOptions::CACHEABLE
            | PagingOptions::USER_ACCESSIBLE,
    );
    Ok(())
}
