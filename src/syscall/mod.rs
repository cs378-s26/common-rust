pub mod numbers;
pub mod fs;
pub mod process;
pub mod net;
pub mod legacy;

use alloc::sync::Arc;
pub use numbers::number;
pub use numbers::wrapper_constants::*;

use crate::thread::Thread;
use self::fs::*;
use self::process::*;
use self::net::*;
#[cfg(target_arch = "x86_64")]
use self::legacy::*;

// SyscallContext Trait
// The purpose of this trait is to unify system calls between
// x86 and ARM such that each architecture can implement this
// trait for their respective interrupt context structs.
// Thus, the syscall handler will only have to deal with a
// struct implementing SycallContext, agnostic of architecture

pub trait SyscallContext {
    // syscall number
    // x8 on ARM - rax on x86
    fn syscall_number(&self) -> u64;

    // max of 6 arguments to a unix sycall
    fn arg0(&self) -> u64;
    fn arg1(&self) -> u64;
    fn arg2(&self) -> u64;
    fn arg3(&self) -> u64;
    fn arg4(&self) -> u64;
    fn arg5(&self) -> u64;

    fn get_arg(&self, n: u64) -> Option<u64> {
        match n {
            0 => Some(self.arg0()),
            1 => Some(self.arg1()),
            2 => Some(self.arg2()),
            3 => Some(self.arg3()),
            4 => Some(self.arg4()),
            5 => Some(self.arg5()),
            _ => None,
        }
    }

    fn set_return_value(&mut self, ret: u64);

    fn is_user_address(&self, ptr: u64) -> bool;

    fn get_arg_ptr_safe(&self, n: u64) -> Option<u64> {
        let potential_pointer = self.get_arg(n)?;
        if self.is_user_address(potential_pointer) {
            Some(potential_pointer)
        } else {
            None
        }
    }
}

pub fn syscall_handler(thread: &Arc<Thread>, ctx: &mut impl SyscallContext) {
    let num = ctx.syscall_number();

    match num {
        // Modern core ABI (Aarch64 uses these exclusively while x86_64 uses both because of legacy support)
        number::READ => {
            ctx.set_return_value(sys_read(thread, ctx));
        }
        number::WRITE => {
            ctx.set_return_value(sys_write(thread, ctx));
        }
        number::OPENAT => {
            ctx.set_return_value(sys_openat(thread, ctx));
        }
        number::CLOSE => {
            ctx.set_return_value(sys_close(thread, ctx));
        }
        number::CLONE => {
            ctx.set_return_value(sys_clone(thread, ctx));
        }
        number::PIPE2 => {
            ctx.set_return_value(sys_pipe2(thread, ctx));
        }
        number::NEWFSTATAT => {
            ctx.set_return_value(sys_newfstatat(thread, ctx));
        }
        number::PPOLL => {
            ctx.set_return_value(sys_ppoll(thread, ctx));
        }
        number::FACCESSAT => {
            ctx.set_return_value(sys_faccessat(thread, ctx));
        }
        number::PSELECT6 => {
            ctx.set_return_value(sys_pselect6(thread, ctx));
        }
        number::MKDIRAT => {
            ctx.set_return_value(sys_mkdirat(thread, ctx));
        }
        number::UNLINKAT => {
            ctx.set_return_value(sys_unlinkat(thread, ctx));
        }
        number::EXIT => {
            ctx.set_return_value(sys_exit(thread, ctx));
        }
        number::GETPID => {
            ctx.set_return_value(sys_getpid(thread, ctx));
        }

        // x86_64 libraries will use these legacy system calls which ARM does not support any more
        // they can all be transformed to be satisfied by calls to the more modern functions via wrappers
        #[cfg(target_arch = "x86_64")]
        number::OPEN => {
            ctx.set_return_value(sys_open_wrapper(thread, ctx));
        }
        #[cfg(target_arch = "x86_64")]
        number::STAT => {
            ctx.set_return_value(sys_stat_wrapper(thread, ctx));
        }
        #[cfg(target_arch = "x86_64")]
        number::LSTAT => {
            ctx.set_return_value(sys_lstat_wrapper(thread, ctx));
        }
        #[cfg(target_arch = "x86_64")]
        number::POLL => {
            ctx.set_return_value(sys_poll_wrapper(thread, ctx));
        }
        #[cfg(target_arch = "x86_64")]
        number::ACCESS => {
            ctx.set_return_value(sys_access_wrapper(thread, ctx));
        }
        #[cfg(target_arch = "x86_64")]
        number::PIPE => {
            ctx.set_return_value(sys_pipe_wrapper(thread, ctx));
        }
        #[cfg(target_arch = "x86_64")]
        number::SELECT => {
            ctx.set_return_value(sys_select_wrapper(thread, ctx));
        }
        #[cfg(target_arch = "x86_64")]
        number::FORK => {
            ctx.set_return_value(sys_fork_wrapper(thread, ctx));
        }
        #[cfg(target_arch = "x86_64")]
        number::VFORK => {
            ctx.set_return_value(sys_vfork_wrapper(thread, ctx));
        }
        #[cfg(target_arch = "x86_64")]
        number::MKDIR => {
            ctx.set_return_value(sys_mkdir_wrapper(thread, ctx));
        }
        #[cfg(target_arch = "x86_64")]
        number::RMDIR => {
            ctx.set_return_value(sys_rmdir_wrapper(thread, ctx));
        }
        #[cfg(target_arch = "x86_64")]
        number::UNLINK => {
            ctx.set_return_value(sys_unlink_wrapper(thread, ctx));
        }

        _ => panic!("SYSCALL {} UNIMPLEMENTED", num),
    }
}
