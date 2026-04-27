#[cfg(target_arch = "x86_64")]
mod x86_64;

#[cfg(target_arch = "x86_64")]
pub use self::x86_64::*;

#[cfg(target_arch = "aarch64")]
mod aarch64;

use alloc::{boxed::Box, vec::Vec};

use limine::{mp::Cpu, request::MpRequest};
use spin::{MutexGuard, Once};

#[cfg(target_arch = "aarch64")]
pub use self::aarch64::*;
use crate::{
    devices::discovery::DeviceDiscovery,
    memory::{dma::MmioRegion, virtual_memory::PagingOptions},
    mp::CoreId,
    print::CharSink,
};

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
    fn new_uthread(pc: u64, sp: u64) -> Self;
}

pub trait ArchTrait {
    type Context: ContextTrait<Arch = Self>;
    fn page_size() -> usize;
    /// returns true if this cpu is the bootstrap processor
    fn is_bsp(req: &MpRequest, cpu: &Cpu) -> bool;

    /// does per core init
    /// this looks like:
    /// 1. setting up the cpu local ptr
    /// 2. setting up tables and interrupts
    /// 3. turning on needed features
    /// # Safety
    /// Should only be called from bootstrap processor during kernel initialization
    unsafe fn initialize_core(cpu: &Cpu) -> ();

    fn set_irq_enabled(enabled: bool);
    fn irq_is_enabled() -> bool;
    fn sleep_core();
    fn wake_other_cores();
    // `handler` will be called on a core upon receiving an IRQ of `irq_num`
    // `handler` will run in interrupt context. You cannot block, and `handler` should
    // really run in O(1) time
    // Acknowledging the interrupt (eg via eoi) is the responsibility of `handler`. For x86-64,
    // we will still re-enable interrupts via setting rflags
    fn register_irq_handler(
        irq_num: u8,
        handler: Box<dyn (Fn() -> Option<()>) + Send + Sync>,
        irq_vec: Option<u8>,
    );

    /*
     *  Same interface as register_irq_handler.
     */
    fn register_msix_handler(
        msix_table: &MmioRegion,
        table_off: u16,
        irq_vec: Option<u8>,
        handler: Box<dyn (Fn() -> Option<()>) + Send + Sync>,
    );

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

    fn read_cycle_counter() -> u64;
    const PAGE_SIZE: usize;
    fn get_kernel_address_space() -> u64;
    fn get_user_address_space() -> u64;
    fn set_user_address_space(space: u64);
    fn configure_vm();
    fn virtual_map(space: u64, vaddr: u64, paddr: u64, options: PagingOptions);
    fn virtual_unmap(space: u64, vaddr: u64) -> Option<u64>;
    /// Unmaps a page without freeing the physical frame (for MMIO / externally-owned backing).
    fn virtual_unmap_no_dealloc(space: u64, vaddr: u64) -> Option<u64>;
    fn virtual_invalidate(vaddr: u64);
    fn shootdown_tlbs(space: u64, base: usize, length: usize);
    fn shutdown(err_code: u16);
    fn halt() -> !;
    fn create_arch_specific_drivers(
        system_drivers: &mut Vec<Box<dyn DeviceDiscovery + Send + Sync>>,
    );

    fn init_tty(cell: &Once<Box<dyn CharSink>>);
}
