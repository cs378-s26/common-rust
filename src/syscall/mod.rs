pub mod numbers;
use alloc::{sync::Arc, vec, vec::Vec};

pub use numbers::number;
#[cfg(target_arch = "x86_64")]
pub use numbers::wrapper_constants::*;

use crate::{
    devices::discovery::NETWORK_DEVICES,
    net::{
        Ipv4Addr,
        arp::{ARP_TABLE, build_arp_request},
        ethernet::{EtherType, MacAddr, build_ethernet_frame},
        ipv4::{Protocol, build_ipv4_packet},
        socket::UdpSocket,
        udp::{UDP_DEMUX, UdpSink, build_udp_packet},
    },
    sync::MutexLike,
    thread::Thread,
};

const EBADF: i64 = -9;
const EFAULT: i64 = -14;
const EIO: i64 = -5;
const EINVAL: i64 = -22;
const ENOTSOCK: i64 = -88;

// SycallContext Trait
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
            let ret = sys_read(thread, ctx, ctx.arg0(), ctx.arg1(), ctx.arg2());
            ctx.set_return_value(ret as u64);
        }
        number::WRITE => {
            let ret = sys_write(thread, ctx, ctx.arg0(), ctx.arg1(), ctx.arg2());
            ctx.set_return_value(ret as u64);
        }
        number::OPENAT => {
            sys_openat(ctx.arg0() as i32, ctx.arg1(), ctx.arg2(), ctx.arg3());
        }
        number::CLOSE => {
            let ret = sys_close(thread, ctx.arg0() as i32);
            ctx.set_return_value(ret as u64);
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
        number::SOCKET => {
            let ret = sys_socket(thread, ctx.arg0(), ctx.arg1(), ctx.arg2());
            ctx.set_return_value(ret as u64);
        }
        number::BIND => {
            let ret = sys_bind(thread, ctx, ctx.arg0(), ctx.arg1(), ctx.arg2());
            ctx.set_return_value(ret as u64);
        }
        number::SENDTO => {
            let ret = sys_sendto(
                thread, ctx,
                ctx.arg0(), ctx.arg1(), ctx.arg2(),
                ctx.arg3(), ctx.arg4(), ctx.arg5(),
            );
            ctx.set_return_value(ret as u64);
        }
        number::RECVFROM => {
            let ret = sys_recvfrom(
                thread, ctx,
                ctx.arg0(), ctx.arg1(), ctx.arg2(),
                ctx.arg3(), ctx.arg4(), ctx.arg5(),
            );
            ctx.set_return_value(ret as u64);
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

// Write kernel data into a user-space buffer.
unsafe fn copy_to_user(dst_ptr: u64, src: &[u8]) {
    unsafe { core::ptr::copy_nonoverlapping(src.as_ptr(), dst_ptr as *mut u8, src.len()); }
}

// Read user-space data into a kernel buffer.
unsafe fn copy_from_user(src_ptr: u64, dst: &mut [u8]) {
    unsafe { core::ptr::copy_nonoverlapping(src_ptr as *const u8, dst.as_mut_ptr(), dst.len()); }
}

fn sys_read(thread: &Arc<Thread>, ctx: &impl SyscallContext, fd: u64, buf_ptr: u64, count: u64) -> i64 {
    let Some(process) = thread.process.get() else { return EBADF; };
    let Some(file) = process.fd_table.lock().get(fd as i32) else { return EBADF; };
    if !ctx.is_user_address(buf_ptr) {
        return EFAULT;
    }
    let mut kernel_buf: Vec<u8> = vec![0u8; count as usize];
    match file.read(&mut kernel_buf) {
        Ok(n) => {
            unsafe { copy_to_user(buf_ptr, &kernel_buf[..n]); }
            n as i64
        }
        Err(_) => EIO,
    }
}

fn sys_write(thread: &Arc<Thread>, ctx: &impl SyscallContext, fd: u64, buf_ptr: u64, count: u64) -> i64 {
    let Some(process) = thread.process.get() else { return EBADF; };
    let Some(file) = process.fd_table.lock().get(fd as i32) else { return EBADF; };
    if !ctx.is_user_address(buf_ptr) {
        return EFAULT;
    }
    let mut kernel_buf: Vec<u8> = vec![0u8; count as usize];
    unsafe { copy_from_user(buf_ptr, &mut kernel_buf); }
    match file.write(&kernel_buf) {
        Ok(n) => n as i64,
        Err(_) => EIO,
    }
}

fn sys_openat(_dirfd: i32, _pathname: u64, _flags: u64, _mode: u64) {}
fn sys_close(thread: &Arc<Thread>, fd: i32) -> i64 {
    let Some(process) = thread.process.get() else { return EBADF; };
    let Some(file) = process.fd_table.lock().remove(fd) else { return EBADF; };
    match file.close() {
        Ok(()) => 0,
        Err(_) => EIO,
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

fn sys_socket(thread: &Arc<Thread>, domain: u64, type_: u64, protocol: u64) -> i64 {
    const AF_INET: u64 = 2;
    const SOCK_DGRAM: u64 = 2;
    const SOCK_TYPE_MASK: u64 = 0xf;

    if domain != AF_INET {
        return EINVAL;
    }
    if type_ & SOCK_TYPE_MASK != SOCK_DGRAM {
        return EINVAL;
    }
    // 0 means "default for type" (UDP for SOCK_DGRAM); 17 is IPPROTO_UDP
    if protocol != 0 && protocol != 17 {
        return EINVAL;
    }

    let Some(process) = thread.process.get() else { return EBADF; };
    let socket = UdpSocket::new();
    let fd = process.fd_table.lock().insert(socket);
    fd as i64
}

fn sys_bind(thread: &Arc<Thread>, ctx: &impl SyscallContext, fd: u64, sockaddr_ptr: u64, _addrlen: u64) -> i64 {
    const AF_INET: u16 = 2;

    if !ctx.is_user_address(sockaddr_ptr) {
        return EFAULT;
    }

    // sockaddr_in: sin_family (2, native-endian) | sin_port (2, big-endian) | sin_addr (4, big-endian)
    let mut buf = [0u8; 8];
    unsafe { copy_from_user(sockaddr_ptr, &mut buf); }

    let family = u16::from_ne_bytes([buf[0], buf[1]]);
    if family != AF_INET {
        return EINVAL;
    }

    let port = u16::from_be_bytes([buf[2], buf[3]]);
    let ip = Ipv4Addr([buf[4], buf[5], buf[6], buf[7]]);

    let Some(process) = thread.process.get() else { return EBADF; };
    let Some(file) = process.fd_table.lock().get(fd as i32) else { return EBADF; };

    let Ok(socket) = file.as_any_arc().downcast::<UdpSocket>() else { return ENOTSOCK; };

    socket.bind(ip, port);
    UDP_DEMUX.register(port, Arc::clone(&socket) as Arc<dyn UdpSink>);

    0
}

fn sys_sendto(
    thread: &Arc<Thread>,
    ctx: &impl SyscallContext,
    fd: u64,
    buf_ptr: u64,
    len: u64,
    _flags: u64,
    dest_addr_ptr: u64,
    _addrlen: u64,
) -> i64 {
    const AF_INET: u16 = 2;
    const BUF_SIZE: usize = 1536;

    if !ctx.is_user_address(buf_ptr) || !ctx.is_user_address(dest_addr_ptr) {
        return EFAULT;
    }

    let Some(process) = thread.process.get() else { return EBADF; };
    let Some(file) = process.fd_table.lock().get(fd as i32) else { return EBADF; };
    let Ok(socket) = file.as_any_arc().downcast::<UdpSocket>() else { return ENOTSOCK; };
    let Some((src_ip, src_port)) = socket.local_addr() else { return EINVAL; };

    // read dest sockaddr_in: family(2) | port(2, BE) | ip(4)
    let mut addr_buf = [0u8; 8];
    unsafe { copy_from_user(dest_addr_ptr, &mut addr_buf); }
    if u16::from_ne_bytes([addr_buf[0], addr_buf[1]]) != AF_INET {
        return EINVAL;
    }
    let dst_port = u16::from_be_bytes([addr_buf[2], addr_buf[3]]);
    let dst_ip = Ipv4Addr([addr_buf[4], addr_buf[5], addr_buf[6], addr_buf[7]]);

    // read payload from user
    let count = len as usize;
    let mut payload = alloc::vec![0u8; count];
    unsafe { copy_from_user(buf_ptr, &mut payload); }

    // get our MAC from the NIC
    let our_mac = {
        let devs = NETWORK_DEVICES.lock();
        match devs.first() {
            Some(nic) => nic.mac_address(),
            None => return EIO,
        }
    };

    // resolve dest MAC — try cache, otherwise send ARP request and block
    let dst_mac = match ARP_TABLE.resolve(dst_ip) {
        Some(mac) => mac,
        None => {
            let promise = ARP_TABLE.start_lookup(dst_ip);

            let mut arp_buf = [0u8; 28];
            let mut frame_buf = [0u8; BUF_SIZE];
            let broadcast = MacAddr([0xff; 6]);
            if let Ok(arp_len) = build_arp_request(our_mac, src_ip, dst_ip, &mut arp_buf) {
                if let Ok(frame_len) = build_ethernet_frame(
                    broadcast, our_mac, EtherType::Arp, &arp_buf[..arp_len], &mut frame_buf,
                ) {
                    let mut devs = NETWORK_DEVICES.lock();
                    if let Some(nic) = devs.first_mut() {
                        let _ = nic.send_packet(&frame_buf[..frame_len]);
                    }
                }
            }

            promise.get()
        }
    };

    // build UDP → IPv4 → Ethernet and send
    let mut udp_buf = [0u8; BUF_SIZE];
    let mut ip_buf = [0u8; BUF_SIZE];
    let mut frame_buf = [0u8; BUF_SIZE];

    let Ok(udp_len) = build_udp_packet(src_ip, dst_ip, src_port, dst_port, &payload, &mut udp_buf)
        else { return EIO; };
    let Ok(ip_len) = build_ipv4_packet(src_ip, dst_ip, Protocol::Udp, &udp_buf[..udp_len], &mut ip_buf)
        else { return EIO; };
    let Ok(frame_len) = build_ethernet_frame(dst_mac, our_mac, EtherType::Ipv4, &ip_buf[..ip_len], &mut frame_buf)
        else { return EIO; };

    {
        let mut devs = NETWORK_DEVICES.lock();
        if let Some(nic) = devs.first_mut() {
            let _ = nic.send_packet(&frame_buf[..frame_len]);
        }
    }

    count as i64
}

fn sys_recvfrom(
    thread: &Arc<Thread>,
    ctx: &impl SyscallContext,
    fd: u64,
    buf_ptr: u64,
    len: u64,
    _flags: u64,
    src_addr_ptr: u64,
    addrlen_ptr: u64,
) -> i64 {
    if !ctx.is_user_address(buf_ptr) {
        return EFAULT;
    }

    let Some(process) = thread.process.get() else { return EBADF; };
    let Some(file) = process.fd_table.lock().get(fd as i32) else { return EBADF; };
    let Ok(socket) = file.as_any_arc().downcast::<UdpSocket>() else { return ENOTSOCK; };

    let datagram = socket.recv_datagram(); // blocks until a packet arrives

    let n = datagram.data.len().min(len as usize);
    unsafe { copy_to_user(buf_ptr, &datagram.data[..n]); }

    // write source address back if the caller provided a non-null pointer
    if src_addr_ptr != 0 && ctx.is_user_address(src_addr_ptr) {
        // sockaddr_in: family(2, NE) | port(2, BE) | ip(4) | zero padding(8)
        let mut addr_buf = [0u8; 16];
        addr_buf[0..2].copy_from_slice(&2u16.to_ne_bytes()); // AF_INET
        addr_buf[2..4].copy_from_slice(&datagram.src_port.to_be_bytes());
        addr_buf[4..8].copy_from_slice(&datagram.src_ip.0);
        unsafe { copy_to_user(src_addr_ptr, &addr_buf); }

        // write the filled address length back if caller provided addrlen pointer
        if addrlen_ptr != 0 && ctx.is_user_address(addrlen_ptr) {
            unsafe { copy_to_user(addrlen_ptr, &16u32.to_ne_bytes()); }
        }
    }

    n as i64
}

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
