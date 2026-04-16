pub mod numbers;
pub use numbers::number;
#[cfg(target_arch = "x86_64")]
pub use numbers::wrapper_constants::*;

// SycallContext Trait
// The purpose of this trait is to unify system calls between
// x86 and ARM such that each architecture can implement this
// trait for their respective interrupt context structs.
// Thus, the syscall handler will only have to deal with a
// struct implementing SycallContext, agnostic of architecture

pub trait SyscallContext {
    // syscall number
    // x8 on ARM - rax on x86
    fn syscall_number(&self) -> usize;

    // max of 6 arguments to a unix sycall
    fn arg0(&self) -> usize;
    fn arg1(&self) -> usize;
    fn arg2(&self) -> usize;
    fn arg3(&self) -> usize;
    fn arg4(&self) -> usize;
    fn arg5(&self) -> usize;

    fn get_arg(&self, n: usize) -> Option<usize> {
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

    fn set_return_value(&self, ret: usize);

    fn is_user_address(&self, ptr: usize) -> bool;

    fn get_arg_ptr_safe(&self, n: usize) -> Option<usize> {
        let potential_pointer = self.get_arg(n)?;
        if self.is_user_address(potential_pointer) {
            Some(potential_pointer)
        } else {
            None
        }
    }
}

pub fn syscall_handler(ctx: &mut impl SyscallContext) {
    let num = ctx.syscall_number();

    match num {
        // Modern core ABI (Aarch64 uses these exclusively while x86_64 uses both because of legacy support)
        number::READ => {
            sys_read(ctx.arg0(), ctx.arg1(), ctx.arg2());
        }
        number::WRITE => {
            sys_write(ctx.arg0(), ctx.arg1(), ctx.arg2());
        }
        number::OPENAT => {
            sys_openat(ctx.arg0() as i32, ctx.arg1(), ctx.arg2(), ctx.arg3());
        }
        number::CLOSE => {
            sys_close(ctx.arg0() as i32);
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

        // x86_64 libraries will use these legacy system calls which ARM does not support any more
        // they can all be transformed to be satisfied by calls to the more modern functions via wrappers
        #[cfg(target_arch = "x86_64")]
        number::OPEN => {
            sys_open_wrapper(ctx);
        }
        #[cfg(target_arch = "x86_64")]
        number::STAT => {
            sys_stat_wrapper(ctx);
        }
        #[cfg(target_arch = "x86_64")]
        number::LSTAT => {
            sys_lstat_wrapper(ctx);
        }
        #[cfg(target_arch = "x86_64")]
        number::POLL => {
            sys_poll_wrapper(ctx);
        }
        #[cfg(target_arch = "x86_64")]
        number::ACCESS => {
            sys_access_wrapper(ctx);
        }
        #[cfg(target_arch = "x86_64")]
        number::PIPE => {
            sys_pipe_wrapper(ctx);
        }
        #[cfg(target_arch = "x86_64")]
        number::SELECT => {
            sys_select_wrapper(ctx);
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
            sys_mkdir_wrapper(ctx);
        }
        #[cfg(target_arch = "x86_64")]
        number::RMDIR => {
            sys_rmdir_wrapper(ctx);
        }
        #[cfg(target_arch = "x86_64")]
        number::UNLINK => {
            sys_unlink_wrapper(ctx);
        }

        _ => panic!("SYSCALL {} UNIMPLEMENTED", num),
    }
}

// functions that we must implement. These are the only ones used by Aarch64 and
// represent modern supersets(?) (supercalls?) of the legacy systemcalls

fn sys_read(_fd: usize, _buf: usize, _count: usize) {}
fn sys_write(_fd: usize, _buf: usize, _count: usize) {}
fn sys_openat(_dirfd: i32, _pathname: usize, _flags: usize, _mode: usize) {}
fn sys_close(_fd: i32) {}
fn sys_clone(_flags: usize, _stack: usize, _parent_tid: usize, _child_tid: usize, _tls: usize) {}
fn sys_pipe2(_pipefd: usize, _flags: i32) {}
fn sys_newfstatat(_dirfd: i32, _pathname: usize, _statbuf: usize, _flags: i32) {}
fn sys_ppoll(_fds: usize, _nfds: usize, _tmo_p: usize, _sigmask: usize) {}
fn sys_faccessat(_dirfd: i32, _pathname: usize, _mode: i32) {}
fn sys_pselect6(
    _nfds: i32,
    _readfds: usize,
    _writefds: usize,
    _exceptfds: usize,
    _timeout: usize,
    _sigmask: usize,
) {
}
fn sys_mkdirat(_dirfd: i32, _pathname: usize, _mode: usize) {}
fn sys_unlinkat(_dirfd: i32, _pathname: usize, _flags: i32) {}

// Legacy x86_64 wrappers for compatibility

#[cfg(target_arch = "x86_64")]
fn sys_open_wrapper(ctx: &impl SyscallContext) {
    // x86_64 open(pathname, flags, mode)
    sys_openat(AT_FDCWD, ctx.arg0(), ctx.arg1(), ctx.arg2());
}

#[cfg(target_arch = "x86_64")]
fn sys_stat_wrapper(ctx: &impl SyscallContext) {
    // x86_64 stat(pathname, statbuf)
    sys_newfstatat(AT_FDCWD, ctx.arg0(), ctx.arg1(), 0);
}

#[cfg(target_arch = "x86_64")]
fn sys_lstat_wrapper(ctx: &impl SyscallContext) {
    // x86_64 lstat(pathname, statbuf)
    sys_newfstatat(AT_FDCWD, ctx.arg0(), ctx.arg1(), AT_SYMLINK_NOFOLLOW);
}

#[cfg(target_arch = "x86_64")]
fn sys_poll_wrapper(ctx: &impl SyscallContext) {
    // x86_64 poll(fds, nfds, timeout)
    // Wrap by passing a NULL sigmask to ppoll.
    // Note: poll uses ms, ppoll uses timespec; full conversion would happen in sys_ppoll
    sys_ppoll(ctx.arg0(), ctx.arg1(), ctx.arg2(), 0);
}

#[cfg(target_arch = "x86_64")]
fn sys_access_wrapper(ctx: &impl SyscallContext) {
    // x86_64 access(pathname, mode)
    sys_faccessat(AT_FDCWD, ctx.arg0(), ctx.arg1() as i32);
}

#[cfg(target_arch = "x86_64")]
fn sys_pipe_wrapper(ctx: &impl SyscallContext) {
    // x86_64 pipe(pipefd)
    sys_pipe2(ctx.arg0(), 0);
}

#[cfg(target_arch = "x86_64")]
fn sys_select_wrapper(ctx: &impl SyscallContext) {
    // x86_64 select(nfds, readfds, writefds, exceptfds, timeout)
    // Map to pselect6 with NULL sigmask.
    sys_pselect6(
        ctx.arg0() as i32,
        ctx.arg1(),
        ctx.arg2(),
        ctx.arg3(),
        ctx.arg4(),
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
fn sys_mkdir_wrapper(ctx: &impl SyscallContext) {
    // x86_64 mkdir(pathname, mode)
    sys_mkdirat(AT_FDCWD, ctx.arg0(), ctx.arg1());
}

#[cfg(target_arch = "x86_64")]
fn sys_rmdir_wrapper(ctx: &impl SyscallContext) {
    // x86_64 rmdir(pathname) -> unlinkat(AT_FDCWD, pathname, AT_REMOVEDIR)
    const AT_REMOVEDIR: i32 = 0x200;
    sys_unlinkat(AT_FDCWD, ctx.arg0(), AT_REMOVEDIR);
}

#[cfg(target_arch = "x86_64")]
fn sys_unlink_wrapper(ctx: &impl SyscallContext) {
    // x86_64 unlink(pathname)
    sys_unlinkat(AT_FDCWD, ctx.arg0(), 0);
}
