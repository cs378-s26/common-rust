#[cfg(target_arch = "x86_64")]
mod x86_64;

#[cfg(target_arch = "x86_64")]
pub use self::x86_64::*;

#[cfg(target_arch = "aarch64")]
mod aarch64;

#[cfg(target_arch = "aarch64")]
pub use self::aarch64::*;

use crate::{kernel_main, mp::CoreId};
use core::sync::atomic::Ordering;
use limine::{mp::Cpu, request::MpRequest};
use spin::MutexGuard;

pub trait UnwindContextTrait {
    /// Returns the current stack frame as an unwind context
    unsafe fn get() -> Self;
    unsafe fn valid(&self) -> bool {
        (unsafe { self.return_address() }) != 0
    }

    unsafe fn return_address(&self) -> u64 {
        unsafe { self.get_ptr().wrapping_add(1).read() }
    }

    fn from_ptr(ptr: *const u64) -> UnwindContext;
    fn get_ptr(&self) -> *const u64;

    unsafe fn next(&self) -> UnwindContext {
        Self::from_ptr(unsafe { self.get_ptr().read() } as *const u64)
    }
}

pub trait IrqStateTrait {
    type Arch: ArchTrait<IrqState = Self>;
    // Save the current IrqState
    fn save() -> Self;
    fn is_masked(&self) -> bool;
    fn restore(&self) {
        Arch::set_irq_enabled(!self.is_masked());
    }
}

pub trait ContextTrait {
    type Arch: ArchTrait<Context = Self>;
    /// from what i understand basically a constructor -- give your thread the correct perms
    fn setup_kthread_context(&mut self);
    fn jump_to(&self) -> !;
    fn setup_for_call<T>(
        &mut self,
        stack: &[u8],
        function: unsafe extern "C" fn(*mut T) -> !,
        data: *mut T,
    );
}

pub trait ArchTrait {
    type Context: ContextTrait<Arch = Self>;
    type IrqState: IrqStateTrait<Arch = Self>;
    /// returns true if this cpu is the bootstrap processor
    fn is_bsp(req: &MpRequest, cpu: &Cpu) -> bool;
    /// calls initalize core
    fn initialize_mp(req: &MpRequest) -> ! {
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
                cpu.goto_address.write(Self::start_core);
            }
        }
        unsafe { Self::start_core(bsp.expect("Couldn't find the bootstrap processor")) }
    }
    /// does per core init
    /// this looks like:
    /// 1. setting up the cpu local ptr
    /// 2. setting up tables and interrupts
    /// 3. turning on needed features
    unsafe fn initialize_core(cpu: &Cpu) -> ();
    /// wrapper around initalize core that goes to kernel main
    unsafe extern "C" fn start_core(cpu: &Cpu) -> ! {
        unsafe { Self::initialize_core(cpu) };
        kernel_main()
    }
    fn set_irq_enabled(enabled: bool);
    /// save the current context and swith on to the provided temp stack & call fwd()
    unsafe fn save_context<T: FnOnce() -> !>(
        temp_stack: &[u8],
        ctx: MutexGuard<'static, Self::Context>,
        fwd: T,
    );
    fn set_cpu_local_pointer(core_id: CoreId);
    fn get_cpu_local_pointer() -> u64;
    fn set_thread_local_pointer(base: *const u64);
    fn get_thread_local_pointer() -> u64;
    fn halt() -> !;
}
