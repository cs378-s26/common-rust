#[cfg(target_arch = "x86_64")]
mod x86_64;

#[cfg(target_arch = "x86_64")]
pub use self::x86_64::*;

#[cfg(target_arch = "aarch64")]
mod aarch64;

#[cfg(target_arch = "aarch64")]
pub use self::aarch64::*;

use crate::kernel_main;
use limine::request::MpRequest;

// pub trait Arch {
//     // the only job of this function is to call the core_entry function for each core
//     fn initialize_mp(req: MpRequest) -> ();
//     // Contract: this function will set the CPU local pointer and set up interrupts
//     unsafe fn initialize_core() -> ();
// }

// unsafe fn start_core<A: Arch>() -> ! {
//     unsafe { A::initialize_core() };
//     kernel_main()
// }

// #[cfg(target_arch = "aarch64")]
// pub unsafe extern "C" fn core_entry() -> ! {
//     unsafe { start_core::<Aarch64>() }
// }

// #[cfg(target_arch = "x86_64")]
// pub unsafe extern "C" fn core_entry() -> ! {
//     unsafe { start_core::<X86_64>() }
// }
