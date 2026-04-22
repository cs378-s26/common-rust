#![cfg(target_arch = "x86_64")]

use alloc::sync::Arc;
use crate::thread::Thread;
use super::SyscallContext;

pub fn sys_open_wrapper(_thread: &Arc<Thread>, _ctx: &impl SyscallContext) -> u64 {
    panic!("legacy open -> openat translation unimplemented");
}

pub fn sys_stat_wrapper(_thread: &Arc<Thread>, _ctx: &impl SyscallContext) -> u64 {
    panic!("legacy stat -> newfstatat translation unimplemented");
}

pub fn sys_lstat_wrapper(_thread: &Arc<Thread>, _ctx: &impl SyscallContext) -> u64 {
    panic!("legacy lstat -> newfstatat translation unimplemented");
}

pub fn sys_poll_wrapper(_thread: &Arc<Thread>, _ctx: &impl SyscallContext) -> u64 {
    panic!("legacy poll -> ppoll translation unimplemented");
}

pub fn sys_access_wrapper(_thread: &Arc<Thread>, _ctx: &impl SyscallContext) -> u64 {
    panic!("legacy access -> faccessat translation unimplemented");
}

pub fn sys_pipe_wrapper(_thread: &Arc<Thread>, _ctx: &impl SyscallContext) -> u64 {
    panic!("legacy pipe -> pipe2 translation unimplemented");
}

pub fn sys_select_wrapper(_thread: &Arc<Thread>, _ctx: &impl SyscallContext) -> u64 {
    panic!("legacy select -> pselect6 translation unimplemented");
}

pub fn sys_fork_wrapper(_thread: &Arc<Thread>, _ctx: &impl SyscallContext) -> u64 {
    panic!("legacy fork -> clone translation unimplemented");
}

pub fn sys_vfork_wrapper(_thread: &Arc<Thread>, _ctx: &impl SyscallContext) -> u64 {
    panic!("legacy vfork -> clone translation unimplemented");
}

pub fn sys_mkdir_wrapper(_thread: &Arc<Thread>, _ctx: &impl SyscallContext) -> u64 {
    panic!("legacy mkdir -> mkdirat translation unimplemented");
}

pub fn sys_rmdir_wrapper(_thread: &Arc<Thread>, _ctx: &impl SyscallContext) -> u64 {
    panic!("legacy rmdir -> unlinkat translation unimplemented");
}

pub fn sys_unlink_wrapper(_thread: &Arc<Thread>, _ctx: &impl SyscallContext) -> u64 {
    panic!("legacy unlink -> unlinkat translation unimplemented");
}
