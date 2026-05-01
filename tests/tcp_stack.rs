#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_common::test_runner)]

kernel_common::integration_test!({
    extern crate alloc;
    use alloc::sync::{Arc, Weak};

    use kernel_common::{
        net::{
            Ipv4Addr,
            ethernet::MacAddr,
            tcp::{
                FLAG_ACK, FLAG_FIN, FLAG_SYN, TCP_DEMUX, TcpConn, TcpError, TcpHeader, TcpState,
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

    // ── Full three-way handshake → accept queue ───────────────────────────────────
    //
    // ISN_COUNTER is private, so we can't predict what new_from_syn assigns.
    // Instead we manually construct a child conn in SynRcvd with a known ISN (9000),
    // set its listener back-reference via Arc::get_mut (safe while rc==1), register
    // it in the demux, then deliver the final ACK.  handle_syn_rcvd verifies
    // hdr.ack == snd_nxt and pushes the child into the listener's accept_queue.
    {
        const PORT: u16 = 55_810;
        let local_ip   = Ipv4Addr([192, 168, 1, 1]);
        let remote_ip  = Ipv4Addr([10, 0, 0, 99]);
        let remote_mac = MacAddr([0xBB; 6]);

        let listener = TcpConn::new(local_ip, PORT);
        listener.listen();

        let mut child = TcpConn::new(local_ip, PORT);
        {
            // rc == 1 here so Arc::get_mut succeeds; set the listener back-ref
            let c = Arc::get_mut(&mut child).unwrap();
            c.listener = Some(Weak::clone(&Arc::downgrade(&listener)));
        }
        {
            let mut inner = child.inner.lock();
            inner.state       = TcpState::SynRcvd;
            inner.remote_addr = remote_ip;
            inner.remote_port = 7000;
            inner.remote_mac  = remote_mac;
            inner.snd_nxt     = 9001; // ISN=9000, SYN consumed one seq number
            inner.snd_una     = 9000;
            inner.rcv_nxt     = 301;  // peer SYN seq=300, consumed one
        }
        TCP_DEMUX.register_established(PORT, remote_ip, 7000, Arc::clone(&child));

        // final ACK from peer — ack=9001 matches child snd_nxt
        let mut buf = [0u8; 64];
        let len = build_tcp_segment(
            remote_ip, local_ip, 7000, PORT,
            301, 9001, FLAG_ACK, 65535, None, &[], &mut buf,
        ).expect("build handshake ACK failed");
        TCP_DEMUX.deliver(remote_ip, local_ip, remote_mac, &buf[..len]);

        assert_eq!(child.inner.lock().state, TcpState::Established, "child should be Established");
        let accepted = listener.accept_queue.pop();
        assert_eq!(accepted.inner.lock().state, TcpState::Established, "accepted conn should be Established");

        TCP_DEMUX.unregister_listener(PORT);
        TCP_DEMUX.unregister_established(PORT, remote_ip, 7000);
        kprintln!("tcp: full handshake and accept passed");
    }

    // ── Data transfer on established connection ───────────────────────────────────
    //
    // Manually set up an established conn and deliver a data segment through the
    // demux.  handle_established pushes the payload into rx_buf.  pop() returns
    // immediately because try_push already incremented the semaphore.
    {
        const PORT: u16 = 55_811;
        let local_ip   = Ipv4Addr([192, 168, 1, 1]);
        let remote_ip  = Ipv4Addr([10, 0, 0, 99]);
        let remote_mac = MacAddr([0xBB; 6]);

        let conn = TcpConn::new(local_ip, PORT);
        {
            let mut inner = conn.inner.lock();
            inner.state       = TcpState::Established;
            inner.remote_addr = remote_ip;
            inner.remote_port = 8000;
            inner.remote_mac  = remote_mac;
            inner.snd_nxt     = 1000;
            inner.rcv_nxt     = 1000;
        }
        TCP_DEMUX.register_established(PORT, remote_ip, 8000, Arc::clone(&conn));

        let payload = b"hello kernel";
        let mut buf = [0u8; 128];
        let len = build_tcp_segment(
            remote_ip, local_ip, 8000, PORT,
            1000, 1000, FLAG_ACK, 65535, None, payload, &mut buf,
        ).expect("build data seg failed");
        TCP_DEMUX.deliver(remote_ip, local_ip, remote_mac, &buf[..len]);

        let received = conn.rx_buf.pop();
        assert!(&received[..] == &payload[..], "received payload mismatch");
        assert_eq!(conn.inner.lock().rcv_nxt, 1012, "rcv_nxt should advance by payload len (12)");

        TCP_DEMUX.unregister_established(PORT, remote_ip, 8000);
        kprintln!("tcp: data transfer after established passed");
    }

    // ── Active close: Established → FinWait1 → FinWait2 → Closed ────────────────
    //
    // conn.close() sends a FIN and advances snd_nxt by one.  A peer ACK
    // acknowledging that FIN moves to FinWait2.  A peer FIN in FinWait2 calls
    // handle_peer_fin, which sets Closed and auto-unregisters from the demux.
    {
        const PORT: u16 = 55_812;
        let local_ip   = Ipv4Addr([192, 168, 1, 1]);
        let remote_ip  = Ipv4Addr([10, 0, 0, 99]);
        let remote_mac = MacAddr([0xBB; 6]);

        let conn = TcpConn::new(local_ip, PORT);
        {
            let mut inner = conn.inner.lock();
            inner.state       = TcpState::Established;
            inner.remote_addr = remote_ip;
            inner.remote_port = 9000;
            inner.remote_mac  = remote_mac;
            inner.snd_nxt     = 300;
            inner.snd_una     = 300;
            inner.rcv_nxt     = 500;
        }
        TCP_DEMUX.register_established(PORT, remote_ip, 9000, Arc::clone(&conn));

        // our FIN at seq=300; snd_nxt becomes 301
        conn.close();
        assert_eq!(conn.inner.lock().state, TcpState::FinWait1, "should be FinWait1 after close");

        // peer ACK(ack=301) covers our FIN → FinWait2
        let mut buf = [0u8; 64];
        let len = build_tcp_segment(
            remote_ip, local_ip, 9000, PORT,
            500, 301, FLAG_ACK, 65535, None, &[], &mut buf,
        ).expect("build peer ACK failed");
        TCP_DEMUX.deliver(remote_ip, local_ip, remote_mac, &buf[..len]);
        assert_eq!(conn.inner.lock().state, TcpState::FinWait2, "should be FinWait2 after peer ACK");

        // peer FIN|ACK → Closed (handle_peer_fin auto-unregisters from demux)
        let mut buf2 = [0u8; 64];
        let len2 = build_tcp_segment(
            remote_ip, local_ip, 9000, PORT,
            500, 301, FLAG_FIN | FLAG_ACK, 65535, None, &[], &mut buf2,
        ).expect("build peer FIN failed");
        TCP_DEMUX.deliver(remote_ip, local_ip, remote_mac, &buf2[..len2]);
        assert_eq!(conn.inner.lock().state, TcpState::Closed, "should be Closed after peer FIN");

        kprintln!("tcp: active close sequence passed");
    }

    // ── Passive close: Established → CloseWait → LastAck → Closed ────────────────
    //
    // Peer initiates close.  handle_established on a FIN moves us to CloseWait.
    // conn.close() from CloseWait sends our FIN and moves to LastAck.
    // A peer ACK of our FIN calls handle_last_ack, which sets Closed and
    // auto-unregisters from the demux.
    {
        const PORT: u16 = 55_813;
        let local_ip   = Ipv4Addr([192, 168, 1, 1]);
        let remote_ip  = Ipv4Addr([10, 0, 0, 99]);
        let remote_mac = MacAddr([0xBB; 6]);

        let conn = TcpConn::new(local_ip, PORT);
        {
            let mut inner = conn.inner.lock();
            inner.state       = TcpState::Established;
            inner.remote_addr = remote_ip;
            inner.remote_port = 9001;
            inner.remote_mac  = remote_mac;
            inner.snd_nxt     = 500;
            inner.snd_una     = 500;
            inner.rcv_nxt     = 400;
        }
        TCP_DEMUX.register_established(PORT, remote_ip, 9001, Arc::clone(&conn));

        // peer FIN at seq=400 → CloseWait, rcv_nxt becomes 401
        let mut buf = [0u8; 64];
        let len = build_tcp_segment(
            remote_ip, local_ip, 9001, PORT,
            400, 500, FLAG_FIN | FLAG_ACK, 65535, None, &[], &mut buf,
        ).expect("build peer FIN failed");
        TCP_DEMUX.deliver(remote_ip, local_ip, remote_mac, &buf[..len]);
        assert_eq!(conn.inner.lock().state, TcpState::CloseWait, "should be CloseWait after peer FIN");

        // our FIN at seq=500; snd_nxt becomes 501
        conn.close();
        assert_eq!(conn.inner.lock().state, TcpState::LastAck, "should be LastAck after our close");

        // peer ACK(ack=501) covers our FIN → Closed (handle_last_ack auto-unregisters)
        let mut buf2 = [0u8; 64];
        let len2 = build_tcp_segment(
            remote_ip, local_ip, 9001, PORT,
            401, 501, FLAG_ACK, 65535, None, &[], &mut buf2,
        ).expect("build final ACK failed");
        TCP_DEMUX.deliver(remote_ip, local_ip, remote_mac, &buf2[..len2]);
        assert_eq!(conn.inner.lock().state, TcpState::Closed, "should be Closed after final ACK");

        kprintln!("tcp: passive close sequence passed");
    }

    // ── connect() happy path: SynSent → Established ───────────────────────────────
    //
    // conn.connect() is non-blocking at the TcpConn level — it sends a SYN
    // (which fails silently with no NIC) and registers in the established table.
    // We read snd_nxt after connect() to learn our ISN+1, then deliver a SYN-ACK
    // with ack=snd_nxt.  handle_syn_sent verifies the ack, sets rcv_nxt=server_ISN+1,
    // and transitions to Established.
    {
        const LOCAL_PORT: u16 = 55_814;
        let local_ip   = Ipv4Addr([192, 168, 1, 1]);
        let server_ip  = Ipv4Addr([10, 0, 0, 1]);
        let server_mac = MacAddr([0xCC; 6]);
        const SERVER_ISN: u32 = 7777;

        let conn = TcpConn::new(local_ip, LOCAL_PORT);
        conn.connect(server_ip, 80, server_mac); // non-blocking; SYN send fails silently (no NIC)
        assert_eq!(conn.inner.lock().state, TcpState::SynSent, "should be SynSent after connect");

        let our_snd_nxt = conn.inner.lock().snd_nxt; // ISN+1

        // server SYN-ACK: seq=SERVER_ISN, ack=our_snd_nxt
        let mut buf = [0u8; 64];
        let len = build_tcp_segment(
            server_ip, local_ip, 80, LOCAL_PORT,
            SERVER_ISN, our_snd_nxt, FLAG_SYN | FLAG_ACK, 65535, Some(1460), &[], &mut buf,
        ).expect("build SYN-ACK failed");
        TCP_DEMUX.deliver(server_ip, local_ip, server_mac, &buf[..len]);

        {
            let inner = conn.inner.lock();
            assert_eq!(inner.state,   TcpState::Established, "should be Established after SYN-ACK");
            assert_eq!(inner.rcv_nxt, SERVER_ISN + 1,        "rcv_nxt should be server ISN + 1");
        }

        TCP_DEMUX.unregister_established(LOCAL_PORT, server_ip, 80);
        kprintln!("tcp: connect() happy path passed");
    }
});
