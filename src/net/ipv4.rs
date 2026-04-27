use spin::Once;

use super::Ipv4Addr;

pub const MIN_HEADER_LEN: usize = 20;

/// protocols we care about in the IPv4 protocol field
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Icmp,
    Tcp,
    Udp,
    Other(u8),
}

impl Protocol {
    fn from_u8(val: u8) -> Self {
        match val {
            1 => Protocol::Icmp,
            6 => Protocol::Tcp,
            17 => Protocol::Udp,
            other => Protocol::Other(other),
        }
    }

    pub fn to_u8(self) -> u8 {
        match self {
            Protocol::Icmp => 1,
            Protocol::Tcp => 6,
            Protocol::Udp => 17,
            Protocol::Other(v) => v,
        }
    }
}

#[derive(Debug)]
pub enum Ipv4Error {
    BufferTooShort,
    OutputBufferTooSmall,
    /// version field isn't 4
    NotIpv4,
    /// header checksum failed
    BadChecksum,
}

/// parsed IPv4 header — payload is a slice into the original buffer
pub struct Ipv4Header<'a> {
    pub src: Ipv4Addr,
    pub dst: Ipv4Addr,
    pub protocol: Protocol,
    pub ttl: u8,
    pub payload: &'a [u8],
}

impl<'a> Ipv4Header<'a> {
    pub fn parse(buf: &'a [u8]) -> Result<Self, Ipv4Error> {
        if buf.len() < MIN_HEADER_LEN {
            return Err(Ipv4Error::BufferTooShort);
        }

        let version = buf[0] >> 4;
        if version != 4 {
            return Err(Ipv4Error::NotIpv4);
        }

        // IHL is in 32-bit words
        let ihl = (buf[0] & 0x0f) as usize * 4;
        if buf.len() < ihl {
            return Err(Ipv4Error::BufferTooShort);
        }

        if checksum(&buf[..ihl]) != 0 {
            return Err(Ipv4Error::BadChecksum);
        }

        let total_len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
        let payload_end = total_len.min(buf.len());

        let ttl = buf[8];
        let protocol = Protocol::from_u8(buf[9]);

        let mut src = [0u8; 4];
        src.copy_from_slice(&buf[12..16]);
        let mut dst = [0u8; 4];
        dst.copy_from_slice(&buf[16..20]);

        Ok(Ipv4Header {
            src: Ipv4Addr(src),
            dst: Ipv4Addr(dst),
            protocol,
            ttl,
            payload: &buf[ihl..payload_end],
        })
    }
}

/// build an outgoing IPv4 packet into `out` — returns total bytes written
/// uses a fixed 20-byte header (no options), DF bit set, TTL 64
pub fn build_ipv4_packet(
    src: Ipv4Addr,
    dst: Ipv4Addr,
    protocol: Protocol,
    payload: &[u8],
    out: &mut [u8],
) -> Result<usize, Ipv4Error> {
    let total = MIN_HEADER_LEN + payload.len();
    if out.len() < total {
        return Err(Ipv4Error::OutputBufferTooSmall);
    }

    out[0] = 0x45; // version=4, IHL=5
    out[1] = 0x00; // DSCP/ECN
    out[2..4].copy_from_slice(&(total as u16).to_be_bytes());
    out[4..6].copy_from_slice(&[0x00, 0x00]); // identification
    out[6..8].copy_from_slice(&[0x40, 0x00]); // DF flag, fragment offset=0
    out[8] = 64; // TTL
    out[9] = protocol.to_u8();
    out[10..12].copy_from_slice(&[0x00, 0x00]); // checksum placeholder
    out[12..16].copy_from_slice(&src.0);
    out[16..20].copy_from_slice(&dst.0);
    out[MIN_HEADER_LEN..total].copy_from_slice(payload);

    // fill in the real checksum
    let cs = checksum(&out[..MIN_HEADER_LEN]);
    out[10..12].copy_from_slice(&cs.to_be_bytes());

    Ok(total)
}

/// one's complement checksum over a byte slice (RFC 791)
/// used for both computing and verifying — a valid header sums to 0
pub fn checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut chunks = data.chunks_exact(2);

    for chunk in chunks.by_ref() {
        sum += u16::from_be_bytes([chunk[0], chunk[1]]) as u32;
    }

    // handle odd byte if any
    if let Some(&last) = chunks.remainder().first() {
        sum += (last as u32) << 8;
    }

    // fold carry bits down to 16 bits
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }

    !(sum as u16)
}

/// our IP configuration — set once at boot, read everywhere
#[derive(Clone, Copy)]
pub struct NetConfig {
    pub ip: Ipv4Addr,
    pub netmask: Ipv4Addr,
    pub gateway: Ipv4Addr,
}

static NET_CONFIG: Once<NetConfig> = Once::new();

pub fn set_net_config(ip: Ipv4Addr, netmask: Ipv4Addr, gateway: Ipv4Addr) {
    NET_CONFIG.call_once(|| NetConfig { ip, netmask, gateway });
}

/// returns None if set_net_config hasn't been called yet
pub fn get_net_config() -> Option<&'static NetConfig> {
    NET_CONFIG.get()
}
