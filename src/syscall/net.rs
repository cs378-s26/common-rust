use alloc::sync::Arc;
use crate::thread::Thread;
use super::SyscallContext;

pub fn sys_pipe2(_thread: &Arc<Thread>, _ctx: &impl SyscallContext) -> u64 {
    -1i64 as u64 // Unimplemented
}

pub fn sys_ppoll(_thread: &Arc<Thread>, _ctx: &impl SyscallContext) -> u64 {
    -1i64 as u64 // Unimplemented
}

pub fn sys_pselect6(_thread: &Arc<Thread>, _ctx: &impl SyscallContext) -> u64 {
    -1i64 as u64 // Unimplemented
}
