/// Length of an Ethernet II header in bytes (dst MAC + src MAC + EtherType).
pub const HEADER_LEN: usize = 14;

/// A 6-byte MAC address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MacAddr(pub [u8; 6]);

/// The EtherType field tells us which network-layer protocol is in the payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EtherType {
    Ipv4,
    Arp,
    Unknown(u16),
}

impl EtherType {
    fn from_u16(val: u16) -> Self {
        match val {
            0x0800 => EtherType::Ipv4,
            0x0806 => EtherType::Arp,
            other => EtherType::Unknown(other),
        }
    }

    pub fn to_u16(self) -> u16 {
        match self {
            EtherType::Ipv4 => 0x0800,
            EtherType::Arp => 0x0806,
            EtherType::Unknown(v) => v,
        }
    }
}

#[derive(Debug)]
pub enum EthernetError {
    /// Incoming buffer is shorter than 14 bytes — can't hold a header.
    BufferTooShort,
    /// Output buffer is too small to hold the header plus payload.
    OutputBufferTooSmall,
}

/// A parsed view of an incoming Ethernet II frame.
/// Borrows the original packet buffer — no copying.
pub struct EthernetFrame<'a> {
    pub dst: MacAddr,
    pub src: MacAddr,
    pub ether_type: EtherType,
    pub payload: &'a [u8],
}

impl<'a> EthernetFrame<'a> {
    /// Parse an Ethernet II header from a raw byte slice.
    /// Returns a frame that borrows the payload portion of `buf`.
    pub fn parse(buf: &'a [u8]) -> Result<Self, EthernetError> {
        if buf.len() < HEADER_LEN {
            return Err(EthernetError::BufferTooShort);
        }

        let mut dst_bytes = [0u8; 6];
        dst_bytes.copy_from_slice(&buf[0..6]);

        let mut src_bytes = [0u8; 6];
        src_bytes.copy_from_slice(&buf[6..12]);

        let ether_type = EtherType::from_u16(u16::from_be_bytes([buf[12], buf[13]]));

        Ok(EthernetFrame {
            dst: MacAddr(dst_bytes),
            src: MacAddr(src_bytes),
            ether_type,
            payload: &buf[HEADER_LEN..],
        })
    }
}

/// Build an outgoing Ethernet II frame into `out`.
/// Returns the total number of bytes written (header + payload length).
pub fn build_ethernet_frame(
    dst: MacAddr,
    src: MacAddr,
    ether_type: EtherType,
    payload: &[u8],
    out: &mut [u8],
) -> Result<usize, EthernetError> {
    let total = HEADER_LEN + payload.len();
    if out.len() < total {
        return Err(EthernetError::OutputBufferTooSmall);
    }

    out[0..6].copy_from_slice(&dst.0);
    out[6..12].copy_from_slice(&src.0);
    let et_bytes = ether_type.to_u16().to_be_bytes();
    out[12] = et_bytes[0];
    out[13] = et_bytes[1];
    out[HEADER_LEN..total].copy_from_slice(payload);

    Ok(total)
}
