#![cfg(target_arch = "x86_64")]

use alloc::sync::Arc;
use crate::thread::Thread;
use super::{SyscallContext, AT_FDCWD};
use super::fs::{do_sys_openat, sys_newfstatat, sys_faccessat, sys_mkdirat, sys_unlinkat};
use super::process::sys_clone;
use super::net::{sys_pipe2, sys_ppoll};

pub fn sys_open_wrapper(thread: &Arc<Thread>, ctx: &impl SyscallContext) -> u64 {
    // x86_64 open(pathname, flags, mode)
    do_sys_openat(AT_FDCWD, ctx.arg0(), ctx.arg1(), ctx.arg2(), thread, ctx)
}

pub fn sys_stat_wrapper(thread: &Arc<Thread>, ctx: &impl SyscallContext) -> u64 {
    // x86_64 stat(pathname, statbuf)
    sys_newfstatat(thread, ctx)
}

pub fn sys_lstat_wrapper(thread: &Arc<Thread>, ctx: &impl SyscallContext) -> u64 {
    // x86_64 lstat(pathname, statbuf)
    sys_newfstatat(thread, ctx)
}

pub fn sys_poll_wrapper(thread: &Arc<Thread>, ctx: &impl SyscallContext) -> u64 {
    // x86_64 poll(fds, nfds, timeout)
    sys_ppoll(thread, ctx)
}

pub fn sys_access_wrapper(thread: &Arc<Thread>, ctx: &impl SyscallContext) -> u64 {
    // x86_64 access(pathname, mode)
    sys_faccessat(thread, ctx)
}

pub fn sys_pipe_wrapper(thread: &Arc<Thread>, ctx: &impl SyscallContext) -> u64 {
    // x86_64 pipe(pipefd)
    sys_pipe2(thread, ctx)
}

pub fn sys_select_wrapper(thread: &Arc<Thread>, ctx: &impl SyscallContext) -> u64 {
    // x86_64 select(nfds, readfds, writefds, exceptfds, timeout)
    super::net::sys_pselect6(thread, ctx)
}

pub fn sys_fork_wrapper(thread: &Arc<Thread>, ctx: &impl SyscallContext) -> u64 {
    // x86_64 fork() -> clone(SIGCHLD, 0, NULL, NULL, 0)
    sys_clone(thread, ctx)
}

pub fn sys_vfork_wrapper(thread: &Arc<Thread>, ctx: &impl SyscallContext) -> u64 {
    // x86_64 vfork() -> clone(CLONE_VFORK | CLONE_VM | SIGCHLD, 0, NULL, NULL, 0)
    sys_clone(thread, ctx)
}

pub fn sys_mkdir_wrapper(thread: &Arc<Thread>, ctx: &impl SyscallContext) -> u64 {
    // x86_64 mkdir(pathname, mode)
    sys_mkdirat(thread, ctx)
}

pub fn sys_rmdir_wrapper(thread: &Arc<Thread>, ctx: &impl SyscallContext) -> u64 {
    // x86_64 rmdir(pathname) -> unlinkat(AT_FDCWD, pathname, AT_REMOVEDIR)
    sys_unlinkat(thread, ctx)
}

pub fn sys_unlink_wrapper(thread: &Arc<Thread>, ctx: &impl SyscallContext) -> u64 {
    // x86_64 unlink(pathname)
    sys_unlinkat(thread, ctx)
}
