#[cfg(target_arch = "x86_64")]
mod x86_64;

#[cfg(target_arch = "x86_64")]
pub use self::x86_64::*;

#[cfg(target_arch = "aarch64")]
mod aarch64;

#[cfg(target_arch = "aarch64")]
pub use self::aarch64::*;

use crate::mp::CoreId;
use crate::virtual_memory::PagingOptions;
use core::sync::atomic::Ordering;
use limine::{mp::Cpu, request::MpRequest};
use spin::MutexGuard;

pub trait UnwindContextTrait: Sized {
    /// Returns the current stack frame as an unwind context
    /// # Safety
    /// Safe if the current function is on a valid stack.
    unsafe fn get() -> Self;

    /// # Safety
    /// Safe if the current function is on a valid stack.
    unsafe fn valid(&self) -> bool {
        (unsafe { self.return_address() }) != 0
    }

    /// # Safety
    /// Safe if the current function is on a valid stack.
    unsafe fn return_address(&self) -> u64 {
        unsafe { self.get_ptr().wrapping_add(1).read() }
    }

    fn from_ptr(ptr: *const u64) -> Self;
    fn get_ptr(&self) -> *const u64;

    /// # Safety
    /// Safe if the current function is on a valid stack.
    unsafe fn next(&self) -> Self {
        Self::from_ptr(unsafe { self.get_ptr().read() } as *const u64)
    }
}

pub trait IrqStateTrait {
    type Arch: ArchTrait<IrqState = Self>;
    // Save the current IrqState
    fn save() -> Self;
    fn is_masked(&self) -> bool;
    fn restore(&self) {
        Self::Arch::set_irq_enabled(!self.is_masked());
    }
}

pub trait ContextTrait {
    type Arch: ArchTrait<Context = Self>;
    /// basically a constructor -- give your thread the correct permissions
    fn setup_kthread_context(&mut self);
    fn jump_to(&self) -> !;
    fn new_kthread<T>(
        stack: &[u8],
        function: unsafe extern "C" fn(*mut T) -> !,
        data: *mut T,
    ) -> Self;
}

pub trait KernelEntryTrait {
    fn kernel_main() -> !;
}

pub trait ArchTrait {
    type Context: ContextTrait<Arch = Self>;
    type IrqState: IrqStateTrait<Arch = Self>;
    /// returns true if this cpu is the bootstrap processor
    fn is_bsp(req: &MpRequest, cpu: &Cpu) -> bool;
    /// calls initalize core
    fn initialize_mp<E: KernelEntryTrait>(req: &MpRequest) -> ! {
        let resp = req
            .get_response()
            .expect("Expected to find MpResponse, got None");
        let mut bsp = None;
        let mut core_id: u64 = 1;
        for cpu in resp.cpus() {
            if Self::is_bsp(req, cpu) {
                bsp = Some(cpu);
            } else {
                cpu.extra.store(core_id, Ordering::SeqCst);
                core_id += 1;
                cpu.goto_address.write(Self::start_core::<E>);
            }
        }
        unsafe { Self::start_core::<E>(bsp.expect("Couldn't find the bootstrap processor")) }
    }
    /// does per core init
    /// this looks like:
    /// 1. setting up the cpu local ptr
    /// 2. setting up tables and interrupts
    /// 3. turning on needed features
    /// # Safety
    /// Should only be called from bootstrap processor during kernel initialization
    unsafe fn initialize_core(cpu: &Cpu) -> ();

    /// wrapper around initalize core that goes to kernel main
    /// # Safety
    /// Should only be called from bootstrap processor during kernel initialization
    unsafe extern "C" fn start_core<E: KernelEntryTrait>(cpu: &Cpu) -> ! {
        unsafe { Self::initialize_core(cpu) };
        E::kernel_main()
    }
    fn set_irq_enabled(enabled: bool);
    fn irq_is_enabled() -> bool;
    /// save the current context and switch on to the provided temp stack & call fwd()
    /// # Safety
    /// Internal, do not call outside of thread module.
    unsafe fn save_context<T: FnOnce() -> !>(
        temp_stack: &[u8],
        ctx: MutexGuard<'static, Self::Context>,
        fwd: T,
    );
    fn set_cpu_local_pointer(core_id: CoreId);
    fn get_cpu_local_pointer() -> u64;

    /// # Safety
    /// Internal, do not call outside of thread module.
    unsafe fn set_thread_local_pointer(base: *const u64);

    /// # Safety
    /// Internal, do not call outside of thread module.
    unsafe fn get_thread_local_pointer() -> u64;

    fn vaddr_to_paddr(space: u64, vaddr: u64) -> Option<u64>;

    fn read_cycle_counter() -> u64;
    fn timer_ticks() -> u64;
    const PAGE_SIZE: usize;
    fn get_address_space() -> u64;
    fn virtual_map(space: u64, vaddr: u64, paddr: u64, options: PagingOptions);
    fn virtual_unmap(space: u64, vaddr: u64) -> Option<u64>;
    fn shutdown(err_code: u16);
    fn halt() -> !;
}
