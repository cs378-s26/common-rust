pub mod numbers;
use alloc::sync::Arc;
use alloc::string::String;
use alloc::vec::Vec;

pub use numbers::number;
pub use numbers::wrapper_constants::*;

use crate::thread::Thread;
use crate::fs::vfs::{VFS, FsError, INodeType};
use crate::fs::file::File;
use crate::sync::MutexLike;
use crate::print::kprint;

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

const O_CREAT: u64 = 64;

fn read_user_string(ptr: u64, ctx: &impl SyscallContext) -> Result<String, &'static str> {
    if !ctx.is_user_address(ptr) {
        return Err("Invalid address");
    }
    let mut s = String::new();
    let mut i = 0;
    loop {
        let addr = ptr + i;
        if !ctx.is_user_address(addr) {
            return Err("Invalid address during string read");
        }
        let c = unsafe { *(addr as *const u8) };
        if c == 0 {
            break;
        }
        s.push(c as char);
        i += 1;
        if i > 4096 {
            return Err("String too long");
        }
    }
    Ok(s)
}

// decode registers into the do_functions. one half of the routes to get a syscall done

fn sys_read(thread: &Arc<Thread>, ctx: &impl SyscallContext) -> u64 {
    do_sys_read(ctx.arg0(), ctx.arg1(), ctx.arg2(), thread, ctx)
}

fn sys_write(thread: &Arc<Thread>, ctx: &impl SyscallContext) -> u64 {
    do_sys_write(ctx.arg0(), ctx.arg1(), ctx.arg2(), thread, ctx)
}

fn sys_openat(thread: &Arc<Thread>, ctx: &impl SyscallContext) -> u64 {
    do_sys_openat(ctx.arg0() as i32, ctx.arg1(), ctx.arg2(), ctx.arg3(), thread, ctx)
}

fn sys_close(_thread: &Arc<Thread>, ctx: &impl SyscallContext) -> u64 {
    do_sys_close(ctx.arg0() as i32, _thread)
}

fn sys_clone(_thread: &Arc<Thread>, _ctx: &impl SyscallContext) -> u64 {
    0 // Unimplemented
}

fn sys_pipe2(_thread: &Arc<Thread>, _ctx: &impl SyscallContext) -> u64 {
    -1i64 as u64 // Unimplemented
}

fn sys_newfstatat(_thread: &Arc<Thread>, _ctx: &impl SyscallContext) -> u64 {
    -1i64 as u64 // Unimplemented
}

fn sys_ppoll(_thread: &Arc<Thread>, _ctx: &impl SyscallContext) -> u64 {
    -1i64 as u64 // Unimplemented
}

fn sys_faccessat(_thread: &Arc<Thread>, _ctx: &impl SyscallContext) -> u64 {
    -1i64 as u64 // Unimplemented
}

fn sys_pselect6(_thread: &Arc<Thread>, _ctx: &impl SyscallContext) -> u64 {
    -1i64 as u64 // Unimplemented
}

fn sys_mkdirat(_thread: &Arc<Thread>, _ctx: &impl SyscallContext) -> u64 {
    -1i64 as u64 // Unimplemented
}

fn sys_unlinkat(_thread: &Arc<Thread>, _ctx: &impl SyscallContext) -> u64 {
    -1i64 as u64 // Unimplemented
}

fn sys_exit(thread: &Arc<Thread>, ctx: &impl SyscallContext) -> u64 {
    let exit_code = ctx.arg0() as i32;
    thread.process.get().unwrap().exit_code.set(exit_code);
    0
}

fn sys_getpid(thread: &Arc<Thread>, _ctx: &impl SyscallContext) -> u64 {
    thread.process.get().unwrap().get_pid() as u64
}

// Core Implementation Layer (do_sys_*) - These take explicit, strongly-typed arguments

fn do_sys_read(fd: u64, buf_ptr: u64, count: u64, thread: &Arc<Thread>, ctx: &impl SyscallContext) -> u64 {
    let fd_table = thread.fd_table.lock();
    if let Some(file) = fd_table.get(&(fd as i32)) {
        if !ctx.is_user_address(buf_ptr) || (count > 0 && !ctx.is_user_address(buf_ptr + count - 1)) {
            return -1i64 as u64;
        }
        let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, count as usize) };
        match file.read(buf) {
            Ok(n) => n as u64,
            Err(_) => -1i64 as u64,
        }
    } else {
        -1i64 as u64
    }
}

fn do_sys_write(fd: u64, buf_ptr: u64, count: u64, thread: &Arc<Thread>, ctx: &impl SyscallContext) -> u64 {
    if fd == 1 || fd == 2 {
        if !ctx.is_user_address(buf_ptr) || (count > 0 && !ctx.is_user_address(buf_ptr + count - 1)) {
            return -1i64 as u64;
        }
        let buf = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, count as usize) };
        if let Ok(s) = core::str::from_utf8(buf) {
            kprint!("{}", s);
            return count;
        }
    }

    let fd_table = thread.fd_table.lock();
    if let Some(file) = fd_table.get(&(fd as i32)) {
        if !ctx.is_user_address(buf_ptr) || (count > 0 && !ctx.is_user_address(buf_ptr + count - 1)) {
            return -1i64 as u64;
        }
        let buf = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, count as usize) };
        match file.write(buf) {
            Ok(n) => n as u64,
            Err(_) => -1i64 as u64,
        }
    } else {
        -1i64 as u64
    }
}

fn do_sys_openat(dirfd: i32, pathname_ptr: u64, flags: u64, _mode: u64, thread: &Arc<Thread>, ctx: &impl SyscallContext) -> u64 {
    let pathname = match read_user_string(pathname_ptr, ctx) {
        Ok(s) => s,
        Err(_) => {
            return -1i64 as u64;
        }
    };

    let start_node = if pathname.starts_with('/') {
        VFS.get_root().expect("VFS root not set")
    } else if dirfd == AT_FDCWD {
        thread.cwd.lock().clone().unwrap_or_else(|| VFS.get_root().expect("CWD and VFS root not set"))
    } else {
        let fd_table = thread.fd_table.lock();
        match fd_table.get(&dirfd) {
            Some(file) => file.vnode.clone(),
            None => {
                return -1i64 as u64;
            }
        }
    };

    let components: Vec<&str> = pathname.split('/').filter(|s| !s.is_empty()).collect();
    let mut current = start_node;

    if components.is_empty() {
        let file = Arc::new(File::new(current));
        let mut fd_table = thread.fd_table.lock();
        let mut fd = 3;
        while fd_table.contains_key(&fd) { fd += 1; }
        fd_table.insert(fd, file);
        return fd as u64;
    }

    for &comp in &components[..components.len()-1] {
        match current.lookup(comp) {
            Ok(next) => current = next,
            Err(_) => {
                return -1i64 as u64;
            }
        }
    }

    let last_comp = components.last().unwrap();
    let vnode = match current.lookup(last_comp) {
        Ok(vnode) => vnode,
        Err(FsError::NotFound) if (flags & O_CREAT) != 0 => {
            match current.create_child(last_comp, INodeType::File) {
                Ok(vnode) => vnode,
                Err(_) => {
                    return -1i64 as u64;
                }
            }
        }
        Err(_) => {
            return -1i64 as u64;
        }
    };

    let file = Arc::new(File::new(vnode));
    let mut fd_table = thread.fd_table.lock();
    let mut fd = 3;
    while fd_table.contains_key(&fd) { fd += 1; }
    fd_table.insert(fd, file);
    fd as u64
}

fn do_sys_close(fd: i32, thread: &Arc<Thread>) -> u64 {
    let mut fd_table = thread.fd_table.lock();
    if fd_table.remove(&fd).is_some() {
        0
    } else {
        -1i64 as u64
    }
}

// Legacy x86_64 wrappers for compatibility

#[cfg(target_arch = "x86_64")]
fn sys_open_wrapper(thread: &Arc<Thread>, ctx: &impl SyscallContext) -> u64 {
    // x86_64 open(pathname, flags, mode)
    do_sys_openat(AT_FDCWD, ctx.arg0(), ctx.arg1(), ctx.arg2(), thread, ctx)
}

#[cfg(target_arch = "x86_64")]
fn sys_stat_wrapper(_thread: &Arc<Thread>, _ctx: &impl SyscallContext) -> u64 {
    // x86_64 stat(pathname, statbuf)
    -1i64 as u64 // Unimplemented (sys_newfstatat stub)
}

#[cfg(target_arch = "x86_64")]
fn sys_lstat_wrapper(_thread: &Arc<Thread>, _ctx: &impl SyscallContext) -> u64 {
    // x86_64 lstat(pathname, statbuf)
    -1i64 as u64 // Unimplemented (sys_newfstatat stub)
}

#[cfg(target_arch = "x86_64")]
fn sys_poll_wrapper(_thread: &Arc<Thread>, _ctx: &impl SyscallContext) -> u64 {
    // x86_64 poll(fds, nfds, timeout)
    -1i64 as u64 // Unimplemented (sys_ppoll stub)
}

#[cfg(target_arch = "x86_64")]
fn sys_access_wrapper(_thread: &Arc<Thread>, _ctx: &impl SyscallContext) -> u64 {
    // x86_64 access(pathname, mode)
    -1i64 as u64 // Unimplemented (sys_faccessat stub)
}

#[cfg(target_arch = "x86_64")]
fn sys_pipe_wrapper(_thread: &Arc<Thread>, _ctx: &impl SyscallContext) -> u64 {
    // x86_64 pipe(pipefd)
    -1i64 as u64 // Unimplemented (sys_pipe2 stub)
}

#[cfg(target_arch = "x86_64")]
fn sys_select_wrapper(_thread: &Arc<Thread>, _ctx: &impl SyscallContext) -> u64 {
    // x86_64 select(nfds, readfds, writefds, exceptfds, timeout)
    -1i64 as u64 // Unimplemented (sys_pselect6 stub)
}

#[cfg(target_arch = "x86_64")]
fn sys_fork_wrapper(_thread: &Arc<Thread>, _ctx: &impl SyscallContext) -> u64 {
    // x86_64 fork() -> clone(SIGCHLD, 0, NULL, NULL, 0)
    0 // Unimplemented (sys_clone stub)
}

#[cfg(target_arch = "x86_64")]
fn sys_vfork_wrapper(_thread: &Arc<Thread>, _ctx: &impl SyscallContext) -> u64 {
    // x86_64 vfork() -> clone(CLONE_VFORK | CLONE_VM | SIGCHLD, 0, NULL, NULL, 0)
    0 // Unimplemented (sys_clone stub)
}

#[cfg(target_arch = "x86_64")]
fn sys_mkdir_wrapper(_thread: &Arc<Thread>, _ctx: &impl SyscallContext) -> u64 {
    // x86_64 mkdir(pathname, mode)
    -1i64 as u64 // Unimplemented (sys_mkdirat stub)
}

#[cfg(target_arch = "x86_64")]
fn sys_rmdir_wrapper(_thread: &Arc<Thread>, _ctx: &impl SyscallContext) -> u64 {
    // x86_64 rmdir(pathname) -> unlinkat(AT_FDCWD, pathname, AT_REMOVEDIR)
    -1i64 as u64 // Unimplemented (sys_unlinkat stub)
}

#[cfg(target_arch = "x86_64")]
fn sys_unlink_wrapper(_thread: &Arc<Thread>, _ctx: &impl SyscallContext) -> u64 {
    // x86_64 unlink(pathname)
    -1i64 as u64 // Unimplemented (sys_unlinkat stub)
}
