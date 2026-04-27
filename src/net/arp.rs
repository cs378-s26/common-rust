use alloc::{collections::BTreeMap, sync::Arc};
use spin::Lazy;

use crate::sync::{IntMutex, MutexLike, Promise};

use super::{Ipv4Addr, ethernet::MacAddr};

pub const ARP_PACKET_LEN: usize = 28;

const HTYPE_ETHERNET: u16 = 1;
const PTYPE_IPV4: u16 = 0x0800;
const HLEN: u8 = 6;
const PLEN: u8 = 4;
const OP_REQUEST: u16 = 1;
const OP_REPLY: u16 = 2;

#[derive(Debug)]
pub enum ArpError {
    BufferTooShort,
    OutputBufferTooSmall,
    NotArpIpv4Ethernet,
}

#[derive(Debug, Clone)]
pub struct ArpPacket {
    pub operation: u16,
    pub sender_mac: MacAddr,
    pub sender_ip: Ipv4Addr,
    pub target_mac: MacAddr,
    pub target_ip: Ipv4Addr,
}

impl ArpPacket {
    /// parse the 28-byte ARP payload (bytes after the ethernet header)
    pub fn parse(buf: &[u8]) -> Result<Self, ArpError> {
        if buf.len() < ARP_PACKET_LEN {
            return Err(ArpError::BufferTooShort);
        }

        let htype = u16::from_be_bytes([buf[0], buf[1]]);
        let ptype = u16::from_be_bytes([buf[2], buf[3]]);
        let hlen = buf[4];
        let plen = buf[5];

        if htype != HTYPE_ETHERNET || ptype != PTYPE_IPV4 || hlen != HLEN || plen != PLEN {
            return Err(ArpError::NotArpIpv4Ethernet);
        }

        let operation = u16::from_be_bytes([buf[6], buf[7]]);

        let mut sender_mac = [0u8; 6];
        sender_mac.copy_from_slice(&buf[8..14]);
        let mut sender_ip = [0u8; 4];
        sender_ip.copy_from_slice(&buf[14..18]);
        let mut target_mac = [0u8; 6];
        target_mac.copy_from_slice(&buf[18..24]);
        let mut target_ip = [0u8; 4];
        target_ip.copy_from_slice(&buf[24..28]);

        Ok(ArpPacket {
            operation,
            sender_mac: MacAddr(sender_mac),
            sender_ip: Ipv4Addr(sender_ip),
            target_mac: MacAddr(target_mac),
            target_ip: Ipv4Addr(target_ip),
        })
    }

    pub fn is_request(&self) -> bool {
        self.operation == OP_REQUEST
    }

    pub fn is_reply(&self) -> bool {
        self.operation == OP_REPLY
    }
}

fn write_arp_packet(
    op: u16,
    sender_mac: MacAddr,
    sender_ip: Ipv4Addr,
    target_mac: MacAddr,
    target_ip: Ipv4Addr,
    out: &mut [u8],
) -> Result<usize, ArpError> {
    if out.len() < ARP_PACKET_LEN {
        return Err(ArpError::OutputBufferTooSmall);
    }
    out[0..2].copy_from_slice(&HTYPE_ETHERNET.to_be_bytes());
    out[2..4].copy_from_slice(&PTYPE_IPV4.to_be_bytes());
    out[4] = HLEN;
    out[5] = PLEN;
    out[6..8].copy_from_slice(&op.to_be_bytes());
    out[8..14].copy_from_slice(&sender_mac.0);
    out[14..18].copy_from_slice(&sender_ip.0);
    out[18..24].copy_from_slice(&target_mac.0);
    out[24..28].copy_from_slice(&target_ip.0);
    Ok(ARP_PACKET_LEN)
}

/// "i have <our_ip>, my mac is <our_mac>"
pub fn build_arp_reply(
    our_mac: MacAddr,
    our_ip: Ipv4Addr,
    target_mac: MacAddr,
    target_ip: Ipv4Addr,
    out: &mut [u8],
) -> Result<usize, ArpError> {
    write_arp_packet(OP_REPLY, our_mac, our_ip, target_mac, target_ip, out)
}

/// "who has <target_ip>? tell <our_ip> / <our_mac>" — target mac is zeroed (unknown)
pub fn build_arp_request(
    our_mac: MacAddr,
    our_ip: Ipv4Addr,
    target_ip: Ipv4Addr,
    out: &mut [u8],
) -> Result<usize, ArpError> {
    write_arp_packet(OP_REQUEST, our_mac, our_ip, MacAddr([0; 6]), target_ip, out)
}

pub enum ArpAction {
    None,
    /// send this ARP reply — dst_mac is the ethernet destination, payload is the 28-byte body
    SendReply {
        dst_mac: MacAddr,
        payload: [u8; ARP_PACKET_LEN],
    },
}

struct ArpTableState {
    cache: BTreeMap<Ipv4Addr, MacAddr>,
    pending: BTreeMap<Ipv4Addr, Arc<Promise<MacAddr>>>,
}

pub struct ArpTable {
    state: Lazy<IntMutex<ArpTableState>>,
}

impl ArpTable {
    pub const fn new() -> Self {
        ArpTable {
            state: Lazy::new(|| {
                IntMutex::new(ArpTableState {
                    cache: BTreeMap::new(),
                    pending: BTreeMap::new(),
                })
            }),
        }
    }

    /// non-blocking cache check
    pub fn resolve(&self, ip: Ipv4Addr) -> Option<MacAddr> {
        self.state.lock().cache.get(&ip).copied()
    }

    /// register a pending lookup and return a promise that resolves when the reply arrives
    /// multiple threads waiting on the same IP share one promise and wake together
    pub fn start_lookup(&self, ip: Ipv4Addr) -> Arc<Promise<MacAddr>> {
        let mut st = self.state.lock();

        if let Some(&mac) = st.cache.get(&ip) {
            let p = Arc::new(Promise::new());
            p.set(mac);
            return p;
        }

        if let Some(p) = st.pending.get(&ip) {
            return Arc::clone(p);
        }

        let p = Arc::new(Promise::new());
        st.pending.insert(ip, Arc::clone(&p));
        p
    }

    /// process an incoming ARP packet — update cache, wake any pending lookups,
    /// and return SendReply if it was a request for our IP
    pub fn handle_incoming(
        &self,
        packet: &ArpPacket,
        our_ip: Ipv4Addr,
        our_mac: MacAddr,
    ) -> ArpAction {
        // pull the promise out while holding the lock, then set it outside —
        // avoids calling Promise::set (which takes its own lock) while arp table lock is held
        let to_wake = {
            let mut st = self.state.lock();
            st.cache.insert(packet.sender_ip, packet.sender_mac);
            st.pending.remove(&packet.sender_ip)
        };

        if let Some(promise) = to_wake {
            promise.set(packet.sender_mac);
        }

        if packet.is_request() && packet.target_ip == our_ip {
            let mut payload = [0u8; ARP_PACKET_LEN];
            build_arp_reply(our_mac, our_ip, packet.sender_mac, packet.sender_ip, &mut payload)
                .unwrap();
            return ArpAction::SendReply {
                dst_mac: packet.sender_mac,
                payload,
            };
        }

        ArpAction::None
    }
}

pub static ARP_TABLE: ArpTable = ArpTable::new();
