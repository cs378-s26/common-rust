use alloc::sync::Arc;

use super::SyscallContext;
use crate::thread::Thread;

pub fn sys_exit(thread: &Arc<Thread>, ctx: &impl SyscallContext) -> u64 {
    let exit_code = ctx.arg0() as i32;
    thread.process.get().unwrap().exit_code.set(exit_code);
    0
}

pub fn sys_getpid(thread: &Arc<Thread>, _ctx: &impl SyscallContext) -> u64 {
    thread.process.get().unwrap().get_pid() as u64
}

pub fn sys_clone(_thread: &Arc<Thread>, _ctx: &impl SyscallContext) -> u64 {
    0 // Unimplemented
}
