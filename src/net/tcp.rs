use alloc::{collections::BTreeMap, sync::{Arc, Weak}, vec::Vec};
use core::sync::atomic::{AtomicU32, Ordering};

use spin::Lazy;

use crate::{
    devices::discovery::NETWORK_DEVICES,
    sync::{BoundedBuffer, IntMutex, MutexLike, Promise},
};
use super::{
    Ipv4Addr,
    ethernet::{EtherType, MacAddr, build_ethernet_frame},
    ipv4::{Protocol, build_ipv4_packet},
};

pub const MIN_HEADER_LEN: usize = 20;

pub const FLAG_FIN: u8 = 0x01;
pub const FLAG_SYN: u8 = 0x02;
pub const FLAG_RST: u8 = 0x04;
pub const FLAG_PSH: u8 = 0x08;
pub const FLAG_ACK: u8 = 0x10;

#[derive(Debug)]
pub enum TcpError {
    BufferTooShort,
    OutputBufferTooSmall,
    BadChecksum,
}

/// parsed TCP header — payload borrows the original buffer
pub struct TcpHeader<'a> {
    pub src_port: u16,
    pub dst_port: u16,
    pub seq: u32,
    pub ack: u32,
    pub flags: u8,
    pub window: u16,
    /// MSS value from options if present in a SYN
    pub mss: Option<u16>,
    pub payload: &'a [u8],
}

impl<'a> TcpHeader<'a> {
    pub fn parse(buf: &'a [u8], src: Ipv4Addr, dst: Ipv4Addr) -> Result<Self, TcpError> {
        if buf.len() < MIN_HEADER_LEN {
            return Err(TcpError::BufferTooShort);
        }

        let data_offset = ((buf[12] >> 4) as usize) * 4;
        if buf.len() < data_offset {
            return Err(TcpError::BufferTooShort);
        }

        if tcp_checksum(src, dst, buf) != 0 {
            return Err(TcpError::BadChecksum);
        }

        let src_port = u16::from_be_bytes([buf[0], buf[1]]);
        let dst_port = u16::from_be_bytes([buf[2], buf[3]]);
        let seq = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
        let ack = u32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]);
        let flags = buf[13];
        let window = u16::from_be_bytes([buf[14], buf[15]]);

        let mss = parse_mss_option(&buf[MIN_HEADER_LEN..data_offset]);

        Ok(TcpHeader { src_port, dst_port, seq, ack, flags, window, mss, payload: &buf[data_offset..] })
    }

    pub fn is_syn(&self) -> bool { self.flags & FLAG_SYN != 0 }
    pub fn is_ack(&self) -> bool { self.flags & FLAG_ACK != 0 }
    pub fn is_fin(&self) -> bool { self.flags & FLAG_FIN != 0 }
    pub fn is_rst(&self) -> bool { self.flags & FLAG_RST != 0 }
}

/// build a TCP segment into out — pass mss: Some(val) only for SYN and SYN-ACK
pub fn build_tcp_segment(
    src: Ipv4Addr,
    dst: Ipv4Addr,
    src_port: u16,
    dst_port: u16,
    seq: u32,
    ack: u32,
    flags: u8,
    window: u16,
    mss: Option<u16>,
    payload: &[u8],
    out: &mut [u8],
) -> Result<usize, TcpError> {
    // MSS option is 4 bytes — header grows to 24 bytes (data offset = 6 words)
    let header_len = if mss.is_some() { 24 } else { MIN_HEADER_LEN };
    let total = header_len + payload.len();
    if out.len() < total {
        return Err(TcpError::OutputBufferTooSmall);
    }

    out[0..2].copy_from_slice(&src_port.to_be_bytes());
    out[2..4].copy_from_slice(&dst_port.to_be_bytes());
    out[4..8].copy_from_slice(&seq.to_be_bytes());
    out[8..12].copy_from_slice(&ack.to_be_bytes());
    out[12] = ((header_len / 4) as u8) << 4; // data offset in upper nibble
    out[13] = flags;
    out[14..16].copy_from_slice(&window.to_be_bytes());
    out[16..18].copy_from_slice(&[0x00, 0x00]); // checksum placeholder
    out[18..20].copy_from_slice(&[0x00, 0x00]); // urgent pointer = 0

    if let Some(mss_val) = mss {
        out[20] = 2; // option kind MSS
        out[21] = 4; // option length 4 bytes
        out[22..24].copy_from_slice(&mss_val.to_be_bytes());
    }

    out[header_len..total].copy_from_slice(payload);

    let cs = tcp_checksum(src, dst, &out[..total]);
    out[16..18].copy_from_slice(&cs.to_be_bytes());

    Ok(total)
}

/// scan TCP option bytes for kind=2 (MSS) — ignore everything else
fn parse_mss_option(options: &[u8]) -> Option<u16> {
    let mut i = 0;
    while i < options.len() {
        match options[i] {
            0 => break,       // end of option list
            1 => { i += 1; } // NOP — no length byte
            kind => {
                if i + 1 >= options.len() {
                    break;
                }
                let len = options[i + 1] as usize;
                if len < 2 || i + len > options.len() {
                    break; // malformed option
                }
                if kind == 2 && len == 4 {
                    return Some(u16::from_be_bytes([options[i + 2], options[i + 3]]));
                }
                i += len;
            }
        }
    }
    None
}

/// one's complement checksum over the TCP pseudo-header + TCP segment (RFC 793)
fn tcp_checksum(src: Ipv4Addr, dst: Ipv4Addr, tcp_segment: &[u8]) -> u16 {
    let tcp_len = tcp_segment.len() as u16;

    // 12-byte pseudo-header — src_ip dst_ip zero proto=6 tcp_len
    let pseudo = [
        src.0[0], src.0[1], src.0[2], src.0[3],
        dst.0[0], dst.0[1], dst.0[2], dst.0[3],
        0,
        6,
        (tcp_len >> 8) as u8,
        tcp_len as u8,
    ];

    let mut sum: u32 = 0;

    for chunk in pseudo.chunks_exact(2) {
        sum += u16::from_be_bytes([chunk[0], chunk[1]]) as u32;
    }

    let mut chunks = tcp_segment.chunks_exact(2);
    for chunk in chunks.by_ref() {
        sum += u16::from_be_bytes([chunk[0], chunk[1]]) as u32;
    }
    if let Some(&last) = chunks.remainder().first() {
        sum += (last as u32) << 8;
    }

    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }

    !(sum as u16)
}

// --- ISN counter ---

static ISN_COUNTER: AtomicU32 = AtomicU32::new(1);

fn next_isn() -> u32 {
    ISN_COUNTER.fetch_add(100, Ordering::Relaxed)
}

// --- TCB (Transmission Control Block) ---

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TcpState {
    Closed,
    Listen,
    SynSent,
    SynRcvd,
    Established,
    FinWait1,
    FinWait2,
    CloseWait,
    LastAck,
}

pub struct TcbInner {
    pub state: TcpState,
    pub local_addr: Ipv4Addr,
    pub local_port: u16,
    pub remote_addr: Ipv4Addr,
    pub remote_port: u16,
    pub remote_mac: MacAddr, // filled in when handshake completes, used to send FIN/data
    pub snd_nxt: u32, // next seq number we will send
    pub snd_una: u32, // oldest seq number not yet acked by the peer
    pub rcv_nxt: u32, // next seq number we expect from the peer
    pub rcv_wnd: u16, // receive window we advertise to the peer
    pub mss: u16,     // max segment size, negotiated during the SYN exchange
}

const RX_QUEUE_DEPTH: usize = 32;
const ACCEPT_QUEUE_DEPTH: usize = 8;

/// one TCP connection — from handshake through close
///
/// inner is locked by both the receive loop and syscalls — keep critical
/// sections short. rx_buf, accept_queue, and connected live outside the lock
/// so threads blocked on recv/accept/connect do not prevent the receive loop
/// from making progress
pub struct TcpConn {
    pub inner: IntMutex<TcbInner>,
    pub rx_buf: BoundedBuffer<Vec<u8>, RX_QUEUE_DEPTH>,
    // fully-established connections waiting for accept — only used by listeners
    pub accept_queue: BoundedBuffer<Arc<TcpConn>, ACCEPT_QUEUE_DEPTH>,
    // back-reference to the listening socket that spawned this connection
    pub listener: Option<Weak<TcpConn>>,
    // resolved by handle_syn_sent when the SYN-ACK arrives — connect() blocks on this
    pub connected: Arc<Promise<()>>,
}

impl TcpConn {
    pub fn new(local_addr: Ipv4Addr, local_port: u16) -> Arc<Self> {
        Arc::new(Self {
            inner: IntMutex::new(TcbInner {
                state: TcpState::Closed,
                local_addr,
                local_port,
                remote_addr: Ipv4Addr([0, 0, 0, 0]),
                remote_port: 0,
                remote_mac: MacAddr([0; 6]),
                snd_nxt: 0,
                snd_una: 0,
                rcv_nxt: 0,
                rcv_wnd: 65535,
                mss: 1460,
            }),
            rx_buf: BoundedBuffer::new(),
            accept_queue: BoundedBuffer::new(),
            listener: None,
            connected: Arc::new(Promise::new()),
        })
    }

    // create a SYN_RCVD connection in response to an incoming SYN
    fn new_from_syn(listener: &Arc<TcpConn>, hdr: &TcpHeader<'_>, src_ip: Ipv4Addr, src_mac: MacAddr) -> Arc<Self> {
        let isn = next_isn();
        let local_port = listener.inner.lock().local_port;
        let local_addr = listener.inner.lock().local_addr;
        let mss = hdr.mss.unwrap_or(536);
        Arc::new(Self {
            inner: IntMutex::new(TcbInner {
                state: TcpState::SynRcvd,
                local_addr,
                local_port,
                remote_addr: src_ip,
                remote_port: hdr.src_port,
                remote_mac: src_mac,
                snd_nxt: isn.wrapping_add(1), // SYN consumes one seq number
                snd_una: isn,
                rcv_nxt: hdr.seq.wrapping_add(1), // SYN consumes one seq number
                rcv_wnd: 65535,
                mss,
            }),
            rx_buf: BoundedBuffer::new(),
            accept_queue: BoundedBuffer::new(),
            listener: Some(Arc::downgrade(listener)),
            connected: Arc::new(Promise::new()),
        })
    }

    pub fn local_port(&self) -> u16 {
        self.inner.lock().local_port
    }

    /// returns (remote_addr, remote_port) — only meaningful in ESTABLISHED or later
    pub fn remote(&self) -> (Ipv4Addr, u16) {
        let g = self.inner.lock();
        (g.remote_addr, g.remote_port)
    }

    /// transition to LISTEN and register with the demux table
    pub fn listen(self: &Arc<Self>) {
        self.inner.lock().state = TcpState::Listen;
        TCP_DEMUX.register_listener(self.inner.lock().local_port, Arc::clone(self));
    }

    pub fn handle_segment(self: &Arc<Self>, src_ip: Ipv4Addr, src_mac: MacAddr, hdr: &TcpHeader<'_>) {
        let state = self.inner.lock().state;
        match state {
            TcpState::Listen      => self.handle_listen(src_ip, src_mac, hdr),
            TcpState::SynRcvd    => self.handle_syn_rcvd(src_ip, src_mac, hdr),
            TcpState::SynSent    => self.handle_syn_sent(src_ip, src_mac, hdr),
            TcpState::Established => self.handle_established(src_ip, src_mac, hdr),
            TcpState::FinWait1   => self.handle_fin_wait1(src_ip, src_mac, hdr),
            TcpState::FinWait2   => self.handle_fin_wait2(src_ip, src_mac, hdr),
            TcpState::LastAck    => self.handle_last_ack(hdr),
            _ => {}
        }
    }

    /// initiate an outbound connection — sends SYN and moves to SYN_SENT
    /// caller should wait on self.connected.get() after calling this
    pub fn connect(self: &Arc<Self>, dst_ip: Ipv4Addr, dst_port: u16, dst_mac: MacAddr) {
        let (local_addr, local_port, isn) = {
            let mut inner = self.inner.lock();
            inner.state       = TcpState::SynSent;
            inner.remote_addr = dst_ip;
            inner.remote_port = dst_port;
            inner.remote_mac  = dst_mac;
            let isn = next_isn();
            inner.snd_nxt = isn.wrapping_add(1);
            inner.snd_una = isn;
            (inner.local_addr, inner.local_port, isn)
        };

        TCP_DEMUX.register_established(local_port, dst_ip, dst_port, Arc::clone(self));

        send_segment(
            local_addr, dst_ip, dst_mac,
            local_port, dst_port,
            isn, 0, FLAG_SYN, 65535, Some(1460), &[],
        );
    }

    /// send a FIN and begin the close sequence
    pub fn close(&self) {
        let (local_addr, remote_addr, remote_mac, local_port, remote_port, snd_nxt, rcv_nxt) = {
            let mut inner = self.inner.lock();
            inner.state = match inner.state {
                TcpState::Established => TcpState::FinWait1,
                TcpState::CloseWait   => TcpState::LastAck,
                _ => return,
            };
            let seq = inner.snd_nxt;
            inner.snd_nxt = inner.snd_nxt.wrapping_add(1); // FIN consumes one seq number
            (inner.local_addr, inner.remote_addr, inner.remote_mac,
             inner.local_port, inner.remote_port, seq, inner.rcv_nxt)
        };

        send_segment(
            local_addr, remote_addr, remote_mac,
            local_port, remote_port,
            snd_nxt, rcv_nxt, FLAG_FIN | FLAG_ACK, 65535, None, &[],
        );
    }

    fn handle_listen(self: &Arc<Self>, src_ip: Ipv4Addr, src_mac: MacAddr, hdr: &TcpHeader<'_>) {
        if !hdr.is_syn() {
            return;
        }

        let child = TcpConn::new_from_syn(self, hdr, src_ip, src_mac);
        let child_inner = child.inner.lock();
        let local_addr  = child_inner.local_addr;
        let local_port  = child_inner.local_port;
        let remote_port = child_inner.remote_port;
        let snd_nxt     = child_inner.snd_nxt;
        let rcv_nxt     = child_inner.rcv_nxt;
        let mss         = child_inner.mss;
        drop(child_inner);

        // register before sending SYN-ACK so any immediate ACK can be routed
        TCP_DEMUX.register_established(local_port, src_ip, remote_port, Arc::clone(&child));

        send_segment(
            local_addr, src_ip, src_mac,
            local_port, remote_port,
            snd_nxt.wrapping_sub(1), // seq = ISN
            rcv_nxt,                 // ack = SYN seq + 1
            FLAG_SYN | FLAG_ACK,
            65535,
            Some(mss),
            &[],
        );
    }

    fn handle_syn_rcvd(self: &Arc<Self>, _src_ip: Ipv4Addr, _src_mac: MacAddr, hdr: &TcpHeader<'_>) {
        if !hdr.is_ack() {
            return;
        }

        {
            let mut inner = self.inner.lock();
            // verify the ACK covers our SYN
            if hdr.ack != inner.snd_nxt {
                return;
            }
            inner.snd_una = hdr.ack;
            inner.state   = TcpState::Established;
        }

        // push to the listener's accept queue so accept() can return this conn
        if let Some(weak) = &self.listener {
            if let Some(listener) = weak.upgrade() {
                // try_push because we are in the receive loop and cannot block
                let _ = listener.accept_queue.try_push(Arc::clone(self));
            }
        }
    }

    fn handle_syn_sent(&self, src_ip: Ipv4Addr, src_mac: MacAddr, hdr: &TcpHeader<'_>) {
        if !(hdr.is_syn() && hdr.is_ack()) {
            return;
        }

        let (local_addr, local_port, remote_port, snd_nxt, rcv_nxt) = {
            let mut inner = self.inner.lock();
            if hdr.ack != inner.snd_nxt {
                return;
            }
            inner.snd_una    = hdr.ack;
            inner.rcv_nxt    = hdr.seq.wrapping_add(1);
            inner.remote_mac = src_mac;
            inner.state      = TcpState::Established;
            if let Some(mss) = hdr.mss { inner.mss = mss; }
            (inner.local_addr, inner.local_port, inner.remote_port, inner.snd_nxt, inner.rcv_nxt)
        };

        send_segment(
            local_addr, src_ip, src_mac,
            local_port, remote_port,
            snd_nxt, rcv_nxt, FLAG_ACK, 65535, None, &[],
        );

        self.connected.set(());
    }

    fn handle_established(&self, src_ip: Ipv4Addr, src_mac: MacAddr, hdr: &TcpHeader<'_>) {
        let fin = hdr.is_fin();

        if !hdr.payload.is_empty() || fin {
            let (local_addr, local_port, remote_port, snd_nxt, new_rcv_nxt) = {
                let mut inner = self.inner.lock();
                let payload_len = hdr.payload.len() as u32;
                let fin_len = if fin { 1u32 } else { 0 };
                inner.rcv_nxt = inner.rcv_nxt.wrapping_add(payload_len + fin_len);
                if fin { inner.state = TcpState::CloseWait; }
                (inner.local_addr, inner.local_port, inner.remote_port,
                 inner.snd_nxt, inner.rcv_nxt)
            };

            // ACK the data and/or FIN
            send_segment(
                local_addr, src_ip, src_mac,
                local_port, remote_port,
                snd_nxt, new_rcv_nxt, FLAG_ACK, 65535, None, &[],
            );

            if !hdr.payload.is_empty() {
                // silently drop if full — no retransmission in happy path
                let _ = self.rx_buf.try_push(Vec::from(hdr.payload));
            }
        }
    }

    fn handle_fin_wait1(&self, src_ip: Ipv4Addr, src_mac: MacAddr, hdr: &TcpHeader<'_>) {
        if hdr.is_ack() {
            let mut inner = self.inner.lock();
            inner.snd_una = hdr.ack;
            if hdr.ack == inner.snd_nxt {
                inner.state = TcpState::FinWait2;
            }
        }

        // peer may send FIN at the same time as ACKing ours — handle it
        if hdr.is_fin() {
            self.handle_peer_fin(src_ip, src_mac);
        }
    }

    fn handle_fin_wait2(&self, src_ip: Ipv4Addr, src_mac: MacAddr, hdr: &TcpHeader<'_>) {
        if hdr.is_fin() {
            self.handle_peer_fin(src_ip, src_mac);
        }
    }

    // peer sent FIN while we are in FIN_WAIT_1 or FIN_WAIT_2 — ACK it and close
    fn handle_peer_fin(&self, src_ip: Ipv4Addr, src_mac: MacAddr) {
        let (local_addr, local_port, remote_port, snd_nxt, rcv_nxt) = {
            let mut inner = self.inner.lock();
            inner.rcv_nxt = inner.rcv_nxt.wrapping_add(1); // FIN consumes one seq number
            inner.state   = TcpState::Closed;
            (inner.local_addr, inner.local_port, inner.remote_port,
             inner.snd_nxt, inner.rcv_nxt)
        };

        send_segment(
            local_addr, src_ip, src_mac,
            local_port, remote_port,
            snd_nxt, rcv_nxt, FLAG_ACK, 65535, None, &[],
        );

        let inner = self.inner.lock();
        TCP_DEMUX.unregister_established(inner.local_port, inner.remote_addr, inner.remote_port);
    }

    fn handle_last_ack(&self, hdr: &TcpHeader<'_>) {
        if !hdr.is_ack() {
            return;
        }
        let (acked, local_port, remote_addr, remote_port) = {
            let mut inner = self.inner.lock();
            let acked = hdr.ack == inner.snd_nxt;
            if acked { inner.state = TcpState::Closed; }
            (acked, inner.local_port, inner.remote_addr, inner.remote_port)
        };
        if acked {
            TCP_DEMUX.unregister_established(local_port, remote_addr, remote_port);
        }
    }
}

// --- outbound helper ---

#[allow(clippy::too_many_arguments)]
fn send_segment(
    src_ip: Ipv4Addr,
    dst_ip: Ipv4Addr,
    dst_mac: MacAddr,
    src_port: u16,
    dst_port: u16,
    seq: u32,
    ack: u32,
    flags: u8,
    window: u16,
    mss: Option<u16>,
    payload: &[u8],
) {
    let mut tcp_buf = [0u8; 1536];
    let Ok(tcp_len) = build_tcp_segment(
        src_ip, dst_ip, src_port, dst_port,
        seq, ack, flags, window, mss, payload, &mut tcp_buf,
    ) else { return; };

    let mut ip_buf = [0u8; 1536];
    let Ok(ip_len) = build_ipv4_packet(
        src_ip, dst_ip, Protocol::Tcp, &tcp_buf[..tcp_len], &mut ip_buf,
    ) else { return; };

    let mut frame_buf = [0u8; 1536];
    let our_mac = {
        let devs = NETWORK_DEVICES.lock();
        match devs.first() {
            Some(nic) => nic.mac_address(),
            None => return,
        }
    };
    let Ok(frame_len) = build_ethernet_frame(
        dst_mac, our_mac, EtherType::Ipv4, &ip_buf[..ip_len], &mut frame_buf,
    ) else { return; };

    let mut devs = NETWORK_DEVICES.lock();
    if let Some(nic) = devs.first_mut() {
        let _ = nic.send_packet(&frame_buf[..frame_len]);
    }
}

// --- TCP demux ---

struct TcpDemuxState {
    /// established (and SYN_RCVD) connections keyed by (local_port, remote_ip, remote_port)
    established: BTreeMap<(u16, Ipv4Addr, u16), Arc<TcpConn>>,
    /// listening sockets keyed by local_port
    listeners: BTreeMap<u16, Arc<TcpConn>>,
}

pub struct TcpDemux {
    state: Lazy<IntMutex<TcpDemuxState>>,
}

impl TcpDemux {
    pub const fn new() -> Self {
        TcpDemux {
            state: Lazy::new(|| {
                IntMutex::new(TcpDemuxState {
                    established: BTreeMap::new(),
                    listeners: BTreeMap::new(),
                })
            }),
        }
    }

    pub fn register_listener(&self, port: u16, conn: Arc<TcpConn>) {
        self.state.lock().listeners.insert(port, conn);
    }

    pub fn unregister_listener(&self, port: u16) {
        self.state.lock().listeners.remove(&port);
    }

    pub fn register_established(&self, local_port: u16, remote_addr: Ipv4Addr, remote_port: u16, conn: Arc<TcpConn>) {
        self.state.lock().established.insert((local_port, remote_addr, remote_port), conn);
    }

    pub fn unregister_established(&self, local_port: u16, remote_addr: Ipv4Addr, remote_port: u16) {
        self.state.lock().established.remove(&(local_port, remote_addr, remote_port));
    }

    /// route an incoming TCP segment to the right connection
    /// tries the 4-tuple first (established) then falls back to the listen port
    /// drops silently if neither matches
    pub fn deliver(&self, src_ip: Ipv4Addr, dst_ip: Ipv4Addr, src_mac: MacAddr, payload: &[u8]) {
        let Ok(hdr) = TcpHeader::parse(payload, src_ip, dst_ip) else { return; };

        let conn = {
            let st = self.state.lock();
            let key = (hdr.dst_port, src_ip, hdr.src_port);
            st.established.get(&key)
                .or_else(|| st.listeners.get(&hdr.dst_port))
                .map(Arc::clone)
        };

        if let Some(conn) = conn {
            conn.handle_segment(src_ip, src_mac, &hdr);
        }
    }
}

pub static TCP_DEMUX: TcpDemux = TcpDemux::new();
