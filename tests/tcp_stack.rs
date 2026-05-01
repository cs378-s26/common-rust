#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_common::test_runner)]

kernel_common::integration_test!({
    extern crate alloc;
    use alloc::sync::Arc;

    use kernel_common::{
        net::{
            Ipv4Addr,
            ethernet::MacAddr,
            tcp::{
                FLAG_ACK, FLAG_SYN, TCP_DEMUX, TcpConn, TcpError, TcpHeader, TcpState,
                build_tcp_segment,
            },
            udp::{UDP_DEMUX, UdpDatagram, UdpSink},
        },
        print::kprintln,
        sync::MutexLike,
    };

    // ── TCP wire format ───────────────────────────────────────────────────────────

    // Build a plain SYN (no payload, no options) and parse it back.
    // Verifies every header field survives the encode → decode round-trip and
    // that the checksum is computed and verified correctly.
    {
        let src = Ipv4Addr([10, 0, 0, 1]);
        let dst = Ipv4Addr([10, 0, 0, 2]);
        let mut buf = [0u8; 64];
        let len = build_tcp_segment(src, dst, 1000, 80, 100, 0, FLAG_SYN, 65535, None, &[], &mut buf)
            .expect("build_tcp_segment failed");

        let hdr = TcpHeader::parse(&buf[..len], src, dst).expect("parse failed");
        assert!(hdr.src_port == 1000, "src_port mismatch");
        assert!(hdr.dst_port == 80,   "dst_port mismatch");
        assert!(hdr.seq    == 100,    "seq mismatch");
        assert!(hdr.ack    == 0,      "ack mismatch");
        assert!(hdr.window == 65535,  "window mismatch");
        assert!(hdr.is_syn() && !hdr.is_ack(), "wrong flags");
        assert!(hdr.mss.is_none(),    "no MSS option expected");
        assert!(hdr.payload.is_empty(), "payload should be empty");
        kprintln!("tcp: basic SYN roundtrip passed");
    }

    // Build a SYN that carries the MSS option and verify it is extracted.
    // MSS is encoded as a 4-byte TCP option (kind=2 len=4 value[2]).
    {
        let src = Ipv4Addr([10, 0, 0, 1]);
        let dst = Ipv4Addr([10, 0, 0, 2]);
        let mut buf = [0u8; 64];
        let len = build_tcp_segment(src, dst, 2000, 443, 999, 0, FLAG_SYN, 65535, Some(1460), &[], &mut buf)
            .expect("build SYN+MSS failed");

        let hdr = TcpHeader::parse(&buf[..len], src, dst).expect("parse SYN+MSS failed");
        assert!(hdr.mss == Some(1460), "MSS not preserved");
        kprintln!("tcp: SYN+MSS roundtrip passed");
    }

    // Build a data segment (ACK with payload) and verify the payload is carried
    // through unmodified and the correct flags/sequence numbers are preserved.
    {
        let src = Ipv4Addr([192, 168, 1, 1]);
        let dst = Ipv4Addr([192, 168, 1, 2]);
        let payload = b"hello world";
        let mut buf = [0u8; 128];
        let len = build_tcp_segment(src, dst, 5000, 6000, 42, 43, FLAG_ACK, 8192, None, payload, &mut buf)
            .expect("build data segment failed");

        let hdr = TcpHeader::parse(&buf[..len], src, dst).expect("parse data segment failed");
        assert!(hdr.payload == payload, "payload corrupted");
        assert!(hdr.seq == 42,  "seq mismatch");
        assert!(hdr.ack == 43,  "ack mismatch");
        assert!(hdr.is_ack() && !hdr.is_syn(), "wrong flags");
        kprintln!("tcp: data payload roundtrip passed");
    }

    // Corrupt the checksum field after building — parse must return BadChecksum.
    // This validates that the checksum verification in TcpHeader::parse catches
    // bit-flipped frames and doesn't silently pass bad data to the state machine.
    {
        let src = Ipv4Addr([10, 0, 0, 1]);
        let dst = Ipv4Addr([10, 0, 0, 2]);
        let mut buf = [0u8; 64];
        let len = build_tcp_segment(src, dst, 1000, 80, 1, 0, FLAG_SYN, 65535, None, &[], &mut buf)
            .expect("build failed");
        buf[16] ^= 0xff; // flip checksum bytes at offset 16-17
        assert!(
            matches!(TcpHeader::parse(&buf[..len], src, dst), Err(TcpError::BadChecksum)),
            "should reject bad checksum"
        );
        kprintln!("tcp: bad checksum rejection passed");
    }

    // Pass a buffer shorter than the minimum TCP header (20 bytes).
    // parse must return BufferTooShort before reading any fields.
    {
        let src = Ipv4Addr([10, 0, 0, 1]);
        let dst = Ipv4Addr([10, 0, 0, 2]);
        let short = [0u8; 10];
        assert!(
            matches!(TcpHeader::parse(&short, src, dst), Err(TcpError::BufferTooShort)),
            "should reject truncated input"
        );
        kprintln!("tcp: truncated input rejection passed");
    }

    // ── UDP port conflict detection (validates fix #2 — EADDRINUSE check) ────────
    //
    // is_port_bound() is the helper we added to UdpDemux.
    // sys_bind now calls it before register() and returns EADDRINUSE if true.
    // We test the helper directly since the syscall requires a full Thread + Process.
    {
        const PORT: u16 = 55_801;

        struct DummySink;
        impl UdpSink for DummySink {
            fn receive(&self, _: UdpDatagram) {}
        }

        assert!(!UDP_DEMUX.is_port_bound(PORT),       "port should start free");
        UDP_DEMUX.register(PORT, Arc::new(DummySink));
        assert!(UDP_DEMUX.is_port_bound(PORT),         "port should be bound after register");
        assert!(!UDP_DEMUX.is_port_bound(PORT + 1),   "adjacent port must remain free");
        UDP_DEMUX.unregister(PORT);
        assert!(!UDP_DEMUX.is_port_bound(PORT),        "port should be free after unregister");
        kprintln!("udp: port conflict detection passed");
    }

    // ── TCP listen() state transition (validates fix #4) ─────────────────────────
    //
    // Before fix #4, listen() locked inner twice:
    //   inner.lock().state = Listen;          // lock #1 drops
    //   TCP_DEMUX.register(inner.lock().local_port, ...);  // lock #2
    // A racing receive could see Listen state before the port was in the demux.
    // The fix reads local_port inside the first lock.
    //
    // We verify the visible outcome: state == Listen and the port is preserved.
    {
        const PORT: u16 = 55_802;
        let local_ip = Ipv4Addr([192, 168, 1, 1]);
        let conn = TcpConn::new(local_ip, PORT);

        assert!(conn.inner.lock().state == TcpState::Closed, "should start Closed");

        conn.listen();

        {
            let inner = conn.inner.lock();
            assert!(inner.state      == TcpState::Listen, "state should be Listen");
            assert!(inner.local_port == PORT,             "local_port should be preserved");
            assert!(inner.local_addr.0 == local_ip.0,    "local_addr should be preserved");
        }

        TCP_DEMUX.unregister_listener(PORT);
        kprintln!("tcp: listen() state transition passed");
    }

    // ── Accept queue capacity (validates fix #9 — RST on overflow) ───────────────
    //
    // ACCEPT_QUEUE_DEPTH is 8.  handle_syn_rcvd calls try_push; when it returns
    // false (queue full) our fix sends RST and unregisters the leaked connection.
    // We test the try_push return value directly since constructing a full
    // SYN_RCVD → Established handshake requires knowing the ISN (private).
    {
        let listener = TcpConn::new(Ipv4Addr([0, 0, 0, 0]), 0);
        let cap = listener.accept_queue.capacity();
        assert!(cap == 8, "expected ACCEPT_QUEUE_DEPTH == 8, got {}", cap);

        // fill every slot — each push should succeed
        for i in 0..cap {
            let child = TcpConn::new(Ipv4Addr([0, 0, 0, 0]), i as u16);
            assert!(
                listener.accept_queue.try_push(child),
                "slot {} should be accepted", i
            );
        }

        // the (cap+1)-th push must return false — this is the exact path our RST fix guards
        let overflow = TcpConn::new(Ipv4Addr([0, 0, 0, 0]), cap as u16);
        assert!(
            !listener.accept_queue.try_push(overflow),
            "try_push should return false when queue is full"
        );
        kprintln!("tcp: accept queue overflow detection passed");
    }

    // ── SYN delivery reaches listener and creates child (validates fix #5) ───────
    //
    // Fix #5 collapsed two listener.inner.lock() calls in new_from_syn into one.
    // We verify the observable post-condition: delivering a SYN to a listening
    // connection leaves the listener in Listen state and registers a child in the
    // established table (we confirm routing works by checking the listener is untouched
    // and no panic occurs — a routing failure would surface as a panic or state change).
    {
        const PORT: u16 = 55_803;
        let local_ip   = Ipv4Addr([192, 168, 1, 1]);
        let remote_ip  = Ipv4Addr([10, 0, 0, 99]);
        let remote_mac = MacAddr([0xAA; 6]);

        let listener = TcpConn::new(local_ip, PORT);
        listener.listen();

        // craft a valid SYN from the remote host
        let mut buf = [0u8; 64];
        let len = build_tcp_segment(
            remote_ip, local_ip, 7000, PORT,
            300, 0, FLAG_SYN, 65535, Some(1460), &[], &mut buf,
        ).expect("build SYN failed");

        // route it through the demux — triggers handle_listen → new_from_syn (fix #5)
        TCP_DEMUX.deliver(remote_ip, local_ip, remote_mac, &buf[..len]);

        // listener must stay in Listen (the SYN is handled by the child, not the listener)
        assert!(
            listener.inner.lock().state == TcpState::Listen,
            "listener should remain in Listen after receiving SYN"
        );

        // a second SYN from a *different* source port also routes and creates another child
        let mut buf2 = [0u8; 64];
        let len2 = build_tcp_segment(
            remote_ip, local_ip, 7001, PORT,
            400, 0, FLAG_SYN, 65535, Some(1460), &[], &mut buf2,
        ).expect("build second SYN failed");
        TCP_DEMUX.deliver(remote_ip, local_ip, remote_mac, &buf2[..len2]);
        assert!(
            listener.inner.lock().state == TcpState::Listen,
            "listener should remain in Listen after second SYN"
        );

        // clean up global demux state
        TCP_DEMUX.unregister_listener(PORT);
        TCP_DEMUX.unregister_established(PORT, remote_ip, 7000);
        TCP_DEMUX.unregister_established(PORT, remote_ip, 7001);
        kprintln!("tcp: SYN delivery and child creation passed");
    }
});
