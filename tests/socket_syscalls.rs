#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_common::test_runner)]

kernel_common::integration_test!({
    extern crate alloc;
    use alloc::{sync::Arc, vec};
    use core::sync::atomic::{AtomicBool, Ordering};

    use kernel_common::{
        net::{
            Ipv4Addr,
            arp::{ARP_TABLE, ArpPacket},
            ethernet::MacAddr,
            tcp::TCP_DEMUX,
            udp::{UDP_DEMUX, UdpDatagram},
        },
        print::kprintln,
        process::Process,
        syscall::{SyscallContext, number, syscall_handler},
        thread::{this_thread, yield_thread},
    };

    // ── mock SyscallContext ───────────────────────────────────────────────────────
    //
    // is_user_address always returns true so the syscalls accept kernel heap/stack
    // pointers directly. The real check guards against untrusted user-space pointers;
    // here all pointers are controlled by the test.

    struct Ctx {
        num:  u64,
        args: [u64; 6],
        ret:  u64,
    }

    impl SyscallContext for Ctx {
        fn syscall_number(&self) -> u64          { self.num }
        fn arg0(&self) -> u64                    { self.args[0] }
        fn arg1(&self) -> u64                    { self.args[1] }
        fn arg2(&self) -> u64                    { self.args[2] }
        fn arg3(&self) -> u64                    { self.args[3] }
        fn arg4(&self) -> u64                    { self.args[4] }
        fn arg5(&self) -> u64                    { self.args[5] }
        fn set_return_value(&mut self, ret: u64) { self.ret = ret; }
        fn is_user_address(&self, _ptr: u64) -> bool { true }
    }

    // ── helpers ───────────────────────────────────────────────────────────────────

    // pack a sockaddr_in: family(2 NE) | port(2 BE) | ip(4)
    fn sockaddr_in(ip: [u8; 4], port: u16) -> [u8; 8] {
        let mut buf = [0u8; 8];
        buf[0..2].copy_from_slice(&2u16.to_ne_bytes()); // AF_INET
        buf[2..4].copy_from_slice(&port.to_be_bytes());
        buf[4..8].copy_from_slice(&ip);
        buf
    }

    // ── error constants (matches kernel definitions) ──────────────────────────────

    const EBADF:     i64 = -9;
    const EIO:       i64 = -5;
    const EINVAL:    i64 = -22;
    const ENOTSOCK:  i64 = -88;
    const EADDRINUSE:i64 = -98;

    // ── socket type constants ─────────────────────────────────────────────────────

    const AF_INET:    u64 = 2;
    const SOCK_STREAM:u64 = 1;
    const SOCK_DGRAM: u64 = 2;

    // ── run all tests inside a process-bearing thread ─────────────────────────────
    //
    // syscall_handler requires thread.process to be set. Process::run spawns a
    // thread with the process already attached, which is the correct production path.

    let done = Arc::new(AtomicBool::new(false));
    let done2 = Arc::clone(&done);

    let process = Process::new().expect("failed to create test process");
    Process::run(Arc::clone(&process), move || {
        let thread = this_thread();

        // inline helper so the closure captures `thread`
        let sc = |num: u64, a0: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64| -> i64 {
            let mut ctx = Ctx { num, args: [a0, a1, a2, a3, a4, a5], ret: 0 };
            syscall_handler(&thread, &mut ctx);
            ctx.ret as i64
        };

        // ── sys_socket ────────────────────────────────────────────────────────────
        //
        // Checks: valid UDP/TCP creation, explicit protocol numbers,
        //         invalid domain, invalid type, wrong protocol for each type.

        let udp_fd = sc(number::SOCKET, AF_INET, SOCK_DGRAM, 0, 0, 0, 0);
        assert!(udp_fd >= 0, "socket(DGRAM) should succeed, got {}", udp_fd);

        let tcp_fd = sc(number::SOCKET, AF_INET, SOCK_STREAM, 0, 0, 0, 0);
        assert!(tcp_fd >= 0, "socket(STREAM) should succeed, got {}", tcp_fd);

        // explicit protocol numbers — 17=UDP, 6=TCP
        let fd_tmp = sc(number::SOCKET, AF_INET, SOCK_DGRAM, 17, 0, 0, 0);
        assert!(fd_tmp >= 0, "socket(DGRAM, proto=17) should succeed");
        sc(number::CLOSE, fd_tmp as u64, 0, 0, 0, 0, 0);

        let fd_tmp = sc(number::SOCKET, AF_INET, SOCK_STREAM, 6, 0, 0, 0);
        assert!(fd_tmp >= 0, "socket(STREAM, proto=6) should succeed");
        sc(number::CLOSE, fd_tmp as u64, 0, 0, 0, 0, 0);

        // wrong domain
        assert_eq!(
            sc(number::SOCKET, 0, SOCK_DGRAM, 0, 0, 0, 0), EINVAL,
            "socket(domain=AF_UNSPEC) should return EINVAL"
        );
        assert_eq!(
            sc(number::SOCKET, 10, SOCK_DGRAM, 0, 0, 0, 0), EINVAL,
            "socket(domain=AF_INET6) should return EINVAL (IPv6 not implemented)"
        );

        // wrong type
        assert_eq!(
            sc(number::SOCKET, AF_INET, 99, 0, 0, 0, 0), EINVAL,
            "socket(type=99) should return EINVAL"
        );
        assert_eq!(
            sc(number::SOCKET, AF_INET, 3, 0, 0, 0, 0), EINVAL,
            "socket(SOCK_RAW) should return EINVAL"
        );

        // wrong protocol for DGRAM (6 = TCP, not UDP)
        assert_eq!(
            sc(number::SOCKET, AF_INET, SOCK_DGRAM, 6, 0, 0, 0), EINVAL,
            "socket(DGRAM, proto=IPPROTO_TCP) should return EINVAL"
        );

        // wrong protocol for STREAM (17 = UDP, not TCP)
        assert_eq!(
            sc(number::SOCKET, AF_INET, SOCK_STREAM, 17, 0, 0, 0), EINVAL,
            "socket(STREAM, proto=IPPROTO_UDP) should return EINVAL"
        );

        kprintln!("sys_socket: all tests passed");

        // ── sys_close ─────────────────────────────────────────────────────────────
        //
        // Checks: closing a valid fd succeeds, double-close returns EBADF,
        //         closing an fd that was never opened returns EBADF.

        assert_eq!(sc(number::CLOSE, udp_fd as u64, 0, 0, 0, 0, 0), 0,
            "close(valid fd) should return 0");

        assert_eq!(sc(number::CLOSE, udp_fd as u64, 0, 0, 0, 0, 0), EBADF,
            "close(already closed fd) should return EBADF");

        assert_eq!(sc(number::CLOSE, 9999, 0, 0, 0, 0, 0), EBADF,
            "close(never-opened fd) should return EBADF");

        kprintln!("sys_close: all tests passed");

        // ── sys_bind (UDP) ────────────────────────────────────────────────────────
        //
        // Checks: bind succeeds, port conflict returns EADDRINUSE (fix #2),
        //         wrong address family returns EINVAL, non-socket fd returns ENOTSOCK.

        // fresh socket — udp_fd was closed above so lowest fd is available again
        let udp_fd = sc(number::SOCKET, AF_INET, SOCK_DGRAM, 0, 0, 0, 0);
        assert!(udp_fd >= 0, "need fresh UDP socket for bind tests");

        let addr = sockaddr_in([10, 0, 2, 15], 56_201);
        assert_eq!(
            sc(number::BIND, udp_fd as u64, addr.as_ptr() as u64, 8, 0, 0, 0), 0,
            "bind should succeed on free port"
        );

        // second socket tries same port → EADDRINUSE (validates fix #2)
        let udp2 = sc(number::SOCKET, AF_INET, SOCK_DGRAM, 0, 0, 0, 0);
        let addr_dup = sockaddr_in([10, 0, 2, 15], 56_201);
        assert_eq!(
            sc(number::BIND, udp2 as u64, addr_dup.as_ptr() as u64, 8, 0, 0, 0), EADDRINUSE,
            "bind to an already-bound port should return EADDRINUSE"
        );
        sc(number::CLOSE, udp2 as u64, 0, 0, 0, 0, 0);

        // wrong family (AF_UNSPEC = 0)
        let udp3 = sc(number::SOCKET, AF_INET, SOCK_DGRAM, 0, 0, 0, 0);
        let mut bad_fam = sockaddr_in([10, 0, 2, 15], 56_202);
        bad_fam[0] = 0; bad_fam[1] = 0; // overwrite to AF_UNSPEC
        assert_eq!(
            sc(number::BIND, udp3 as u64, bad_fam.as_ptr() as u64, 8, 0, 0, 0), EINVAL,
            "bind with wrong family should return EINVAL"
        );
        sc(number::CLOSE, udp3 as u64, 0, 0, 0, 0, 0);

        // bind on invalid fd
        assert_eq!(
            sc(number::BIND, 9999, addr.as_ptr() as u64, 8, 0, 0, 0), EBADF,
            "bind on invalid fd should return EBADF"
        );

        kprintln!("sys_bind(UDP): all tests passed");

        // ── sys_bind (TCP) — validates fix #6 ────────────────────────────────────
        //
        // Before fix #6, sys_bind returned ENOTSOCK for TCP sockets. Now it writes
        // ip/port into TcbInner so listen() and connect() have a local address.

        let tcp_addr = sockaddr_in([192, 168, 1, 1], 56_210);
        assert_eq!(
            sc(number::BIND, tcp_fd as u64, tcp_addr.as_ptr() as u64, 8, 0, 0, 0), 0,
            "bind on TCP socket should succeed"
        );
        kprintln!("sys_bind(TCP): test passed");

        // ── sys_listen ────────────────────────────────────────────────────────────
        //
        // Checks: listen transitions TCP socket to Listen state, UDP returns ENOTSOCK,
        //         invalid fd returns EBADF.

        assert_eq!(sc(number::LISTEN, tcp_fd as u64, 8, 0, 0, 0, 0), 0,
            "listen on TCP socket should return 0");

        // listen on a UDP socket → ENOTSOCK
        assert_eq!(sc(number::LISTEN, udp_fd as u64, 8, 0, 0, 0, 0), ENOTSOCK,
            "listen on UDP socket should return ENOTSOCK");

        // listen on invalid fd
        assert_eq!(sc(number::LISTEN, 9999, 8, 0, 0, 0, 0), EBADF,
            "listen on invalid fd should return EBADF");

        kprintln!("sys_listen: all tests passed");

        // ── sys_recvfrom (UDP) ────────────────────────────────────────────────────
        //
        // Injects datagrams via UDP_DEMUX.deliver() so recvfrom returns immediately
        // without needing a real network peer.  Checks: data delivery, source address
        // written back (with addrlen), and truncation when buf is smaller than datagram.

        // inject a 5-byte datagram for port 56_201
        UDP_DEMUX.deliver(UdpDatagram {
            src_ip:   Ipv4Addr([10, 0, 0, 1]),
            src_port: 12345,
            dst_port: 56_201,
            data:     vec![1, 2, 3, 4, 5],
        });

        let mut recv_buf = [0u8; 64];
        let mut src_addr = [0u8; 16];
        let mut addrlen: u32 = 0;
        let addrlen_ptr = (&mut addrlen) as *mut u32 as u64;
        let ret = sc(
            number::RECVFROM,
            udp_fd as u64, recv_buf.as_mut_ptr() as u64, 64,
            0,
            src_addr.as_mut_ptr() as u64, addrlen_ptr,
        );
        assert_eq!(ret, 5, "recvfrom should return byte count (5)");
        assert_eq!(&recv_buf[..5], &[1, 2, 3, 4, 5], "received payload mismatch");
        // verify source address was filled in
        assert_eq!(u16::from_ne_bytes([src_addr[0], src_addr[1]]), 2,
            "src family should be AF_INET=2");
        assert_eq!(u16::from_be_bytes([src_addr[2], src_addr[3]]), 12345,
            "src port mismatch");
        assert_eq!(&src_addr[4..8], &[10, 0, 0, 1], "src IP mismatch");
        assert_eq!(addrlen, 16, "addrlen should be written back as 16 (validates fix #10 analog for recvfrom)");

        // truncation: buf smaller than datagram — returns min(buf_len, data_len)
        UDP_DEMUX.deliver(UdpDatagram {
            src_ip:   Ipv4Addr([10, 0, 0, 2]),
            src_port: 9000,
            dst_port: 56_201,
            data:     vec![10, 20, 30, 40, 50, 60, 70, 80],
        });
        let mut small = [0u8; 4];
        let ret = sc(number::RECVFROM, udp_fd as u64, small.as_mut_ptr() as u64, 4, 0, 0, 0);
        assert_eq!(ret, 4, "recvfrom with 4-byte buf should return 4");
        assert_eq!(&small, &[10, 20, 30, 40], "truncated payload mismatch");

        // null src_addr pointer is accepted (caller doesn't want the address)
        UDP_DEMUX.deliver(UdpDatagram {
            src_ip:   Ipv4Addr([1, 2, 3, 4]),
            src_port: 111,
            dst_port: 56_201,
            data:     vec![0xAB],
        });
        let mut one_byte = [0u8; 4];
        let ret = sc(number::RECVFROM, udp_fd as u64, one_byte.as_mut_ptr() as u64, 4, 0, 0, 0);
        assert_eq!(ret, 1, "recvfrom with null addr_ptr should still return data");
        assert_eq!(one_byte[0], 0xAB);

        kprintln!("sys_recvfrom: all tests passed");

        // ── sys_read (UDP) ────────────────────────────────────────────────────────
        //
        // sys_read on a UDP socket pops the next datagram without recording its source.

        UDP_DEMUX.deliver(UdpDatagram {
            src_ip:   Ipv4Addr([5, 6, 7, 8]),
            src_port: 5000,
            dst_port: 56_201,
            data:     vec![0xAA, 0xBB, 0xCC],
        });
        let mut rbuf = [0u8; 64];
        let ret = sc(number::READ, udp_fd as u64, rbuf.as_mut_ptr() as u64, 64, 0, 0, 0);
        assert_eq!(ret, 3, "read should return 3 bytes");
        assert_eq!(&rbuf[..3], &[0xAA, 0xBB, 0xCC], "read data mismatch");

        // read on invalid fd
        assert_eq!(
            sc(number::READ, 9999, rbuf.as_mut_ptr() as u64, 64, 0, 0, 0), EBADF,
            "read on invalid fd should return EBADF"
        );

        kprintln!("sys_read(UDP): all tests passed");

        // ── sys_read / sys_write buffer cap (MAX_IO = 65536) — validates fix #1 ──
        //
        // Any count > 65536 must return EINVAL immediately, before touching the socket.

        // read with oversized count — does NOT pop from queue (returns EINVAL first)
        UDP_DEMUX.deliver(UdpDatagram {
            src_ip: Ipv4Addr([0; 4]), src_port: 0, dst_port: 56_201, data: vec![99],
        });
        let mut dummy = [0u8; 8];
        assert_eq!(
            sc(number::READ, udp_fd as u64, dummy.as_mut_ptr() as u64, 65537, 0, 0, 0),
            EINVAL,
            "read with count=65537 should return EINVAL"
        );
        // the datagram was NOT consumed — drain it now
        sc(number::READ, udp_fd as u64, dummy.as_mut_ptr() as u64, 8, 0, 0, 0);

        // count == MAX_IO (65536) is allowed
        UDP_DEMUX.deliver(UdpDatagram {
            src_ip: Ipv4Addr([0; 4]), src_port: 0, dst_port: 56_201, data: vec![7],
        });
        let mut exactly = [0u8; 4]; // actual read is min(payload_len, count)
        let ret = sc(number::READ, udp_fd as u64, exactly.as_mut_ptr() as u64, 65536, 0, 0, 0);
        assert_eq!(ret, 1, "read with count=65536 should succeed (returns 1 byte of payload)");

        // write with oversized count
        assert_eq!(
            sc(number::WRITE, udp_fd as u64, dummy.as_ptr() as u64, 65537, 0, 0, 0),
            EINVAL,
            "write with count=65537 should return EINVAL"
        );

        // sendto with oversized count — EINVAL before any network operations
        let dest = sockaddr_in([10, 0, 2, 2], 9000);
        assert_eq!(
            sc(number::SENDTO, udp_fd as u64, dummy.as_ptr() as u64, 65537,
               0, dest.as_ptr() as u64, 8),
            EINVAL,
            "sendto with count=65537 should return EINVAL"
        );

        kprintln!("buffer cap (MAX_IO=65536): all tests passed");

        // ── sys_write (UDP) → EIO ─────────────────────────────────────────────────
        //
        // UdpSocket::write returns FileError::NotSupported — callers must use sendto.

        let wbuf = [1u8, 2, 3, 4];
        assert_eq!(
            sc(number::WRITE, udp_fd as u64, wbuf.as_ptr() as u64, 4, 0, 0, 0),
            EIO,
            "write on UDP socket should return EIO (use sendto)"
        );

        // write on invalid fd
        assert_eq!(
            sc(number::WRITE, 9999, wbuf.as_ptr() as u64, 4, 0, 0, 0),
            EBADF,
            "write on invalid fd should return EBADF"
        );

        kprintln!("sys_write(UDP): tests passed");

        // ── sys_sendto (UDP) ──────────────────────────────────────────────────────
        //
        // Pre-seed the ARP cache so sendto skips the blocking ARP lookup.
        // The packet is built correctly and submitted to the NIC; we don't verify
        // actual delivery (no real network peer needed for these tests).
        //
        // Checks: successful send returns byte count, wrong family → EINVAL,
        //         unbound socket → EINVAL, count > MAX_IO → EINVAL.

        let peer_ip  = Ipv4Addr([10, 0, 2, 2]);
        let peer_mac = MacAddr([0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01]);
        // inject a fake ARP reply — this populates the cache with peer_ip → peer_mac
        // handle_incoming inserts sender_ip/sender_mac regardless of op type
        ARP_TABLE.handle_incoming(
            &ArpPacket {
                operation:  2, // OP_REPLY
                sender_mac: peer_mac,
                sender_ip:  peer_ip,
                target_mac: MacAddr([0; 6]),
                target_ip:  Ipv4Addr([10, 0, 2, 15]),
            },
            Ipv4Addr([10, 0, 2, 15]),
            MacAddr([0; 6]),
        );

        let payload = b"hello";
        let dest    = sockaddr_in([10, 0, 2, 2], 9000);

        // basic send: returns byte count (send errors from NIC are swallowed by the syscall)
        let ret = sc(
            number::SENDTO,
            udp_fd as u64, payload.as_ptr() as u64, payload.len() as u64,
            0, dest.as_ptr() as u64, 8,
        );
        assert_eq!(ret, payload.len() as i64, "sendto should return byte count");

        // wrong dest family
        let mut bad_dest = sockaddr_in([10, 0, 2, 2], 9000);
        bad_dest[0] = 0; bad_dest[1] = 0;
        assert_eq!(
            sc(number::SENDTO, udp_fd as u64, payload.as_ptr() as u64, payload.len() as u64,
               0, bad_dest.as_ptr() as u64, 8),
            EINVAL,
            "sendto with wrong family should return EINVAL"
        );

        // unbound socket has no local addr → EINVAL
        let unbound = sc(number::SOCKET, AF_INET, SOCK_DGRAM, 0, 0, 0, 0);
        assert!(unbound >= 0);
        assert_eq!(
            sc(number::SENDTO, unbound as u64, payload.as_ptr() as u64, payload.len() as u64,
               0, dest.as_ptr() as u64, 8),
            EINVAL,
            "sendto on unbound socket should return EINVAL"
        );
        sc(number::CLOSE, unbound as u64, 0, 0, 0, 0, 0);

        // sendto on invalid fd
        assert_eq!(
            sc(number::SENDTO, 9999, payload.as_ptr() as u64, payload.len() as u64,
               0, dest.as_ptr() as u64, 8),
            EBADF,
            "sendto on invalid fd should return EBADF"
        );

        // sendto on TCP socket → ENOTSOCK
        assert_eq!(
            sc(number::SENDTO, tcp_fd as u64, payload.as_ptr() as u64, payload.len() as u64,
               0, dest.as_ptr() as u64, 8),
            ENOTSOCK,
            "sendto on TCP socket should return ENOTSOCK"
        );

        kprintln!("sys_sendto: all tests passed");

        // ── sys_accept4 ───────────────────────────────────────────────────────────
        //
        // Full accept (blocking on a real incoming connection) requires a network peer,
        // so we test the error paths only. The accept queue capacity and RST-on-overflow
        // behavior are covered by tcp_stack.rs at the TcpConn level.

        // accept4 on invalid fd
        assert_eq!(
            sc(number::ACCEPT4, 9999, 0, 0, 0, 0, 0), EBADF,
            "accept4 on invalid fd should return EBADF"
        );

        // accept4 on a UDP socket → ENOTSOCK
        assert_eq!(
            sc(number::ACCEPT4, udp_fd as u64, 0, 0, 0, 0, 0), ENOTSOCK,
            "accept4 on UDP socket should return ENOTSOCK"
        );

        kprintln!("sys_accept4: error case tests passed");

        // ── sys_connect — error cases ─────────────────────────────────────────────
        //
        // Full connect (blocking on SYN-ACK from a real server) is not tested here.
        // We verify that argument validation fires before any blocking I/O.

        // wrong address family
        let conn_tcp = sc(number::SOCKET, AF_INET, SOCK_STREAM, 0, 0, 0, 0);
        assert!(conn_tcp >= 0);
        let mut bad_addr = sockaddr_in([10, 0, 2, 2], 8080);
        bad_addr[0] = 0; bad_addr[1] = 0; // AF_UNSPEC
        assert_eq!(
            sc(number::CONNECT, conn_tcp as u64, bad_addr.as_ptr() as u64, 8, 0, 0, 0),
            EINVAL,
            "connect with wrong family should return EINVAL"
        );

        // connect on invalid fd
        assert_eq!(
            sc(number::CONNECT, 9999, bad_addr.as_ptr() as u64, 8, 0, 0, 0),
            EBADF,
            "connect on invalid fd should return EBADF"
        );

        sc(number::CLOSE, conn_tcp as u64, 0, 0, 0, 0, 0);
        kprintln!("sys_connect: error case tests passed");

        // ── sys_getpid ────────────────────────────────────────────────────────────

        let pid = sc(number::GETPID, 0, 0, 0, 0, 0, 0);
        assert!(pid > 0, "getpid should return a positive PID, got {}", pid);
        kprintln!("sys_getpid: pid={} test passed", pid);

        // ── fd allocation order ───────────────────────────────────────────────────
        //
        // FdTable always assigns the lowest available fd (POSIX requirement).
        // Open three sockets and verify they get consecutive fds starting from the
        // lowest currently free slot.

        {
            let a = sc(number::SOCKET, AF_INET, SOCK_DGRAM, 0, 0, 0, 0);
            let b = sc(number::SOCKET, AF_INET, SOCK_DGRAM, 0, 0, 0, 0);
            let c = sc(number::SOCKET, AF_INET, SOCK_DGRAM, 0, 0, 0, 0);
            assert!(a >= 0 && b >= 0 && c >= 0, "three socket opens should all succeed");
            assert_eq!(b, a + 1, "fds should be consecutive");
            assert_eq!(c, a + 2, "fds should be consecutive");

            // close the middle one, then open again — should reclaim it
            sc(number::CLOSE, b as u64, 0, 0, 0, 0, 0);
            let b2 = sc(number::SOCKET, AF_INET, SOCK_DGRAM, 0, 0, 0, 0);
            assert_eq!(b2, b, "reopen should reclaim the freed fd");

            sc(number::CLOSE, a as u64, 0, 0, 0, 0, 0);
            sc(number::CLOSE, b2 as u64, 0, 0, 0, 0, 0);
            sc(number::CLOSE, c as u64, 0, 0, 0, 0, 0);
        }
        kprintln!("fd allocation order: test passed");

        // ── sys_listen: unbound socket ────────────────────────────────────────────
        //
        // A socket that was never bound has local_port=0. listen() should still
        // return 0 — the kernel registers port 0 in the demux. Cleanup unregisters
        // port 0 to avoid polluting the demux for subsequent tests.
        {
            let unbound_tcp = sc(number::SOCKET, AF_INET, SOCK_STREAM, 0, 0, 0, 0);
            assert!(unbound_tcp >= 0);
            assert_eq!(
                sc(number::LISTEN, unbound_tcp as u64, 8, 0, 0, 0, 0), 0,
                "listen on unbound TCP socket should succeed"
            );
            TCP_DEMUX.unregister_listener(0);
            sc(number::CLOSE, unbound_tcp as u64, 0, 0, 0, 0, 0);
        }
        kprintln!("sys_listen: unbound socket test passed");

        // ── sys_bind: UDP double-bind on same socket ──────────────────────────────
        //
        // Binding a UDP socket to a port registers it in UDP_DEMUX. A second bind
        // on the same socket to the same port should return EADDRINUSE because
        // the port is already occupied.
        {
            let sock = sc(number::SOCKET, AF_INET, SOCK_DGRAM, 0, 0, 0, 0);
            assert!(sock >= 0);
            let addr1 = sockaddr_in([10, 0, 2, 15], 56_220);
            assert_eq!(
                sc(number::BIND, sock as u64, addr1.as_ptr() as u64, 8, 0, 0, 0), 0,
                "first bind should succeed"
            );
            let addr2 = sockaddr_in([10, 0, 2, 15], 56_220);
            assert_eq!(
                sc(number::BIND, sock as u64, addr2.as_ptr() as u64, 8, 0, 0, 0), EADDRINUSE,
                "second bind to same port on same socket should return EADDRINUSE"
            );
            sc(number::CLOSE, sock as u64, 0, 0, 0, 0, 0);
        }
        kprintln!("sys_bind: double-bind on UDP returned EADDRINUSE");

        // ── sys_bind: TCP double-bind on same socket ──────────────────────────────
        //
        // TCP bind just overwrites the local_addr/local_port in TcbInner — there
        // is no demux registration at bind time, so a second bind always succeeds.
        {
            let sock = sc(number::SOCKET, AF_INET, SOCK_STREAM, 0, 0, 0, 0);
            assert!(sock >= 0);
            let addr1 = sockaddr_in([10, 0, 2, 15], 56_221);
            assert_eq!(
                sc(number::BIND, sock as u64, addr1.as_ptr() as u64, 8, 0, 0, 0), 0,
                "first TCP bind should succeed"
            );
            let addr2 = sockaddr_in([10, 0, 2, 15], 56_222);
            assert_eq!(
                sc(number::BIND, sock as u64, addr2.as_ptr() as u64, 8, 0, 0, 0), 0,
                "second TCP bind should also succeed (overwrites first)"
            );
            sc(number::CLOSE, sock as u64, 0, 0, 0, 0, 0);
        }
        kprintln!("sys_bind: double-bind on TCP socket passed");

        // ── cleanup ───────────────────────────────────────────────────────────────

        sc(number::CLOSE, udp_fd as u64, 0, 0, 0, 0, 0);
        sc(number::CLOSE, tcp_fd as u64, 0, 0, 0, 0, 0);

        kprintln!("all socket syscall tests passed");
        done2.store(true, Ordering::SeqCst);
    });

    // yield until the test thread finishes
    while !done.load(Ordering::SeqCst) {
        yield_thread();
    }
});
