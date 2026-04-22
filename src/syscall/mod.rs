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
            ctx.set_return_value(sys_read(ctx.arg0(), ctx.arg1(), ctx.arg2(), thread, ctx));
        }
        number::WRITE => {
            ctx.set_return_value(sys_write(ctx.arg0(), ctx.arg1(), ctx.arg2(), thread, ctx));
        }
        number::OPENAT => {
            ctx.set_return_value(sys_openat(ctx.arg0() as i32, ctx.arg1(), ctx.arg2(), ctx.arg3(), thread, ctx));
        }
        number::CLOSE => {
            ctx.set_return_value(sys_close(ctx.arg0() as i32, thread));
        }
        number::CLONE => {
            sys_clone(ctx.arg0(), ctx.arg1(), ctx.arg2(), ctx.arg3(), ctx.arg4());
        }
        number::PIPE2 => {
            sys_pipe2(ctx.arg0(), ctx.arg1() as i32);
        }
        number::NEWFSTATAT => {
            sys_newfstatat(ctx.arg0() as i32, ctx.arg1(), ctx.arg2(), ctx.arg3() as i32);
        }
        number::PPOLL => {
            sys_ppoll(ctx.arg0(), ctx.arg1(), ctx.arg2(), ctx.arg3());
        }
        number::FACCESSAT => {
            sys_faccessat(ctx.arg0() as i32, ctx.arg1(), ctx.arg2() as i32);
        }
        number::PSELECT6 => {
            sys_pselect6(
                ctx.arg0() as i32,
                ctx.arg1(),
                ctx.arg2(),
                ctx.arg3(),
                ctx.arg4(),
                ctx.arg5(),
            );
        }
        number::MKDIRAT => {
            sys_mkdirat(ctx.arg0() as i32, ctx.arg1(), ctx.arg2());
        }
        number::UNLINKAT => {
            sys_unlinkat(ctx.arg0() as i32, ctx.arg1(), ctx.arg2() as i32);
        }
        number::EXIT => {
            let exit_code = ctx.arg0() as i32;
            thread.process.get().unwrap().exit_code.set(exit_code);
            // TODO handle thread/process termination and cleanup, needs parent-child relationship most likely
        }
        number::GETPID => {
            ctx.set_return_value(thread.process.get().unwrap().get_pid() as u64);
        }

        // x86_64 libraries will use these legacy system calls which ARM does not support any more
        // they can all be transformed to be satisfied by calls to the more modern functions via wrappers
        #[cfg(target_arch = "x86_64")]
        number::OPEN => {
            sys_open_wrapper(thread, ctx);
        }
        #[cfg(target_arch = "x86_64")]
        number::STAT => {
            sys_stat_wrapper(thread, ctx);
        }
        #[cfg(target_arch = "x86_64")]
        number::LSTAT => {
            sys_lstat_wrapper(thread, ctx);
        }
        #[cfg(target_arch = "x86_64")]
        number::POLL => {
            sys_poll_wrapper(thread, ctx);
        }
        #[cfg(target_arch = "x86_64")]
        number::ACCESS => {
            sys_access_wrapper(thread, ctx);
        }
        #[cfg(target_arch = "x86_64")]
        number::PIPE => {
            sys_pipe_wrapper(thread, ctx);
        }
        #[cfg(target_arch = "x86_64")]
        number::SELECT => {
            sys_select_wrapper(thread, ctx);
        }
        #[cfg(target_arch = "x86_64")]
        number::FORK => {
            sys_fork_wrapper(ctx);
        }
        #[cfg(target_arch = "x86_64")]
        number::VFORK => {
            sys_vfork_wrapper(ctx);
        }
        #[cfg(target_arch = "x86_64")]
        number::MKDIR => {
            sys_mkdir_wrapper(thread, ctx);
        }
        #[cfg(target_arch = "x86_64")]
        number::RMDIR => {
            sys_rmdir_wrapper(thread, ctx);
        }
        #[cfg(target_arch = "x86_64")]
        number::UNLINK => {
            sys_unlink_wrapper(thread, ctx);
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

// functions that we must implement. These are the only ones used by Aarch64 and
// represent modern supersets(?) (supercalls?) of the legacy systemcalls

fn sys_read(fd: u64, buf_ptr: u64, count: u64, thread: &Arc<Thread>, ctx: &impl SyscallContext) -> u64 {
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

fn sys_write(fd: u64, buf_ptr: u64, count: u64, thread: &Arc<Thread>, ctx: &impl SyscallContext) -> u64 {
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

fn sys_openat(dirfd: i32, pathname_ptr: u64, flags: u64, _mode: u64, thread: &Arc<Thread>, ctx: &impl SyscallContext) -> u64 {
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

fn sys_close(fd: i32, thread: &Arc<Thread>) -> u64 {
    let mut fd_table = thread.fd_table.lock();
    if fd_table.remove(&fd).is_some() {
        0
    } else {
        -1i64 as u64
    }
}

fn sys_clone(_flags: u64, _stack: u64, _parent_tid: u64, _child_tid: u64, _tls: u64) {}
fn sys_pipe2(_pipefd: u64, _flags: i32) {}
fn sys_newfstatat(_dirfd: i32, _pathname: u64, _statbuf: u64, _flags: i32) {}
fn sys_ppoll(_fds: u64, _nfds: u64, _tmo_p: u64, _sigmask: u64) {}
fn sys_faccessat(_dirfd: i32, _pathname: u64, _mode: i32) {}
fn sys_pselect6(
    _nfds: i32,
    _readfds: u64,
    _writefds: u64,
    _exceptfds: u64,
    _timeout: u64,
    _sigmask: u64,
) {
}
fn sys_mkdirat(_dirfd: i32, _pathname: u64, _mode: u64) {}
fn sys_unlinkat(_dirfd: i32, _pathname: u64, _flags: i32) {}

// Legacy x86_64 wrappers for compatibility

#[cfg(target_arch = "x86_64")]
fn sys_open_wrapper(thread: &Arc<Thread>, ctx: &mut impl SyscallContext) {
    // x86_64 open(pathname, flags, mode)
    let ret = sys_openat(AT_FDCWD, ctx.arg0(), ctx.arg1(), ctx.arg2(), thread, ctx);
    ctx.set_return_value(ret);
}

#[cfg(target_arch = "x86_64")]
fn sys_stat_wrapper(_thread: &Arc<Thread>, _ctx: &mut impl SyscallContext) {
    // x86_64 stat(pathname, statbuf)
    sys_newfstatat(AT_FDCWD, _ctx.arg0(), _ctx.arg1(), 0);
}

#[cfg(target_arch = "x86_64")]
fn sys_lstat_wrapper(_thread: &Arc<Thread>, _ctx: &mut impl SyscallContext) {
    // x86_64 lstat(pathname, statbuf)
    sys_newfstatat(AT_FDCWD, _ctx.arg0(), _ctx.arg1(), AT_SYMLINK_NOFOLLOW);
}

#[cfg(target_arch = "x86_64")]
fn sys_poll_wrapper(_thread: &Arc<Thread>, _ctx: &mut impl SyscallContext) {
    // x86_64 poll(fds, nfds, timeout)
    // Wrap by passing a NULL sigmask to ppoll.
    // Note: poll uses ms, ppoll uses timespec; full conversion would happen in sys_ppoll
    sys_ppoll(_ctx.arg0(), _ctx.arg1(), _ctx.arg2(), 0);
}

#[cfg(target_arch = "x86_64")]
fn sys_access_wrapper(_thread: &Arc<Thread>, _ctx: &mut impl SyscallContext) {
    // x86_64 access(pathname, mode)
    sys_faccessat(AT_FDCWD, _ctx.arg0(), _ctx.arg1() as i32);
}

#[cfg(target_arch = "x86_64")]
fn sys_pipe_wrapper(_thread: &Arc<Thread>, _ctx: &mut impl SyscallContext) {
    // x86_64 pipe(pipefd)
    sys_pipe2(_ctx.arg0(), 0);
}

#[cfg(target_arch = "x86_64")]
fn sys_select_wrapper(_thread: &Arc<Thread>, _ctx: &mut impl SyscallContext) {
    // x86_64 select(nfds, readfds, writefds, exceptfds, timeout)
    // Map to pselect6 with NULL sigmask.
    sys_pselect6(
        _ctx.arg0() as i32,
        _ctx.arg1(),
        _ctx.arg2(),
        _ctx.arg3(),
        _ctx.arg4(),
        0,
    );
}

#[cfg(target_arch = "x86_64")]
fn sys_fork_wrapper(_ctx: &impl SyscallContext) {
    // x86_64 fork() -> clone(SIGCHLD, 0, NULL, NULL, 0)
    sys_clone(SIGCHLD, 0, 0, 0, 0);
}

#[cfg(target_arch = "x86_64")]
fn sys_vfork_wrapper(_ctx: &impl SyscallContext) {
    // x86_64 vfork() -> clone(CLONE_VFORK | CLONE_VM | SIGCHLD, 0, NULL, NULL, 0)
    sys_clone(CLONE_VFORK | CLONE_VM | SIGCHLD, 0, 0, 0, 0);
}

#[cfg(target_arch = "x86_64")]
fn sys_mkdir_wrapper(_thread: &Arc<Thread>, _ctx: &mut impl SyscallContext) {
    // x86_64 mkdir(pathname, mode)
    sys_mkdirat(AT_FDCWD, _ctx.arg0(), _ctx.arg1());
}

#[cfg(target_arch = "x86_64")]
fn sys_rmdir_wrapper(_thread: &Arc<Thread>, _ctx: &mut impl SyscallContext) {
    // x86_64 rmdir(pathname) -> unlinkat(AT_FDCWD, pathname, AT_REMOVEDIR)
    const AT_REMOVEDIR: i32 = 0x200;
    sys_unlinkat(AT_FDCWD, _ctx.arg0(), AT_REMOVEDIR);
}

#[cfg(target_arch = "x86_64")]
fn sys_unlink_wrapper(_thread: &Arc<Thread>, _ctx: &mut impl SyscallContext) {
    // x86_64 unlink(pathname)
    sys_unlinkat(AT_FDCWD, _ctx.arg0(), 0);
}
