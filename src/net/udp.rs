use alloc::{collections::BTreeMap, sync::Arc, vec::Vec};
use spin::Lazy;

use crate::sync::{IntMutex, MutexLike};

use super::Ipv4Addr;

pub const HEADER_LEN: usize = 8;

#[derive(Debug)]
pub enum UdpError {
    BufferTooShort,
    OutputBufferTooSmall,
    BadChecksum,
}

/// a received UDP datagram with enough context for the socket layer to act on it
pub struct UdpDatagram {
    pub src_ip: Ipv4Addr,
    pub src_port: u16,
    pub dst_port: u16,
    pub data: Vec<u8>,
}

/// parsed UDP header — payload borrows the original buffer
pub struct UdpHeader<'a> {
    pub src_port: u16,
    pub dst_port: u16,
    pub payload: &'a [u8],
}

impl<'a> UdpHeader<'a> {
    /// parse a UDP segment — `src` and `dst` are the IPv4 addresses from the enclosing IP header,
    /// needed to verify the checksum pseudo-header
    pub fn parse(buf: &'a [u8], src: Ipv4Addr, dst: Ipv4Addr) -> Result<Self, UdpError> {
        if buf.len() < HEADER_LEN {
            return Err(UdpError::BufferTooShort);
        }

        let src_port = u16::from_be_bytes([buf[0], buf[1]]);
        let dst_port = u16::from_be_bytes([buf[2], buf[3]]);
        let udp_len = u16::from_be_bytes([buf[4], buf[5]]) as usize;
        let checksum = u16::from_be_bytes([buf[6], buf[7]]);

        let payload_end = udp_len.min(buf.len());

        // checksum of 0 means sender skipped it — allowed in IPv4
        if checksum != 0 && udp_checksum(src, dst, &buf[..payload_end]) != 0 {
            return Err(UdpError::BadChecksum);
        }

        Ok(UdpHeader {
            src_port,
            dst_port,
            payload: &buf[HEADER_LEN..payload_end],
        })
    }
}

/// build a UDP segment into `out` — returns total bytes written (header + payload)
pub fn build_udp_packet(
    src_ip: Ipv4Addr,
    dst_ip: Ipv4Addr,
    src_port: u16,
    dst_port: u16,
    payload: &[u8],
    out: &mut [u8],
) -> Result<usize, UdpError> {
    let total = HEADER_LEN + payload.len();
    if out.len() < total {
        return Err(UdpError::OutputBufferTooSmall);
    }

    let udp_len = total as u16;
    out[0..2].copy_from_slice(&src_port.to_be_bytes());
    out[2..4].copy_from_slice(&dst_port.to_be_bytes());
    out[4..6].copy_from_slice(&udp_len.to_be_bytes());
    out[6..8].copy_from_slice(&[0x00, 0x00]); // checksum placeholder
    out[HEADER_LEN..total].copy_from_slice(payload);

    let cs = udp_checksum(src_ip, dst_ip, &out[..total]);
    out[6..8].copy_from_slice(&cs.to_be_bytes());

    Ok(total)
}

/// one's complement checksum over the UDP pseudo-header + UDP segment (RFC 768)
fn udp_checksum(src: Ipv4Addr, dst: Ipv4Addr, udp_segment: &[u8]) -> u16 {
    let udp_len = udp_segment.len() as u16;

    // 12-byte pseudo-header: src_ip, dst_ip, zero, proto=17, udp_len
    let pseudo = [
        src.0[0], src.0[1], src.0[2], src.0[3],
        dst.0[0], dst.0[1], dst.0[2], dst.0[3],
        0,
        17,
        (udp_len >> 8) as u8,
        udp_len as u8,
    ];

    let mut sum: u32 = 0;

    for chunk in pseudo.chunks_exact(2) {
        sum += u16::from_be_bytes([chunk[0], chunk[1]]) as u32;
    }

    let mut chunks = udp_segment.chunks_exact(2);
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

/// implemented by anything that can receive a UDP datagram (e.g. UdpSocket in phase 3)
pub trait UdpSink: Send + Sync {
    fn receive(&self, datagram: UdpDatagram);
}

struct UdpDemuxState {
    sockets: BTreeMap<u16, Arc<dyn UdpSink>>,
}

pub struct UdpDemux {
    state: Lazy<IntMutex<UdpDemuxState>>,
}

impl UdpDemux {
    pub const fn new() -> Self {
        UdpDemux {
            state: Lazy::new(|| {
                IntMutex::new(UdpDemuxState {
                    sockets: BTreeMap::new(),
                })
            }),
        }
    }

    pub fn is_port_bound(&self, port: u16) -> bool {
        self.state.lock().sockets.contains_key(&port)
    }

    pub fn register(&self, port: u16, sink: Arc<dyn UdpSink>) {
        self.state.lock().sockets.insert(port, sink);
    }

    pub fn unregister(&self, port: u16) {
        self.state.lock().sockets.remove(&port);
    }

    /// hand a datagram to whichever socket is bound to its dst_port, if any
    pub fn deliver(&self, datagram: UdpDatagram) {
        let sink = self.state.lock().sockets.get(&datagram.dst_port).cloned();
        if let Some(sink) = sink {
            sink.receive(datagram);
        }
    }
}

pub static UDP_DEMUX: UdpDemux = UdpDemux::new();
