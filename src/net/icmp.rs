use super::ipv4::checksum;

pub const HEADER_LEN: usize = 8;

const TYPE_ECHO_REQUEST: u8 = 8;
const TYPE_ECHO_REPLY: u8 = 0;

#[derive(Debug)]
pub enum IcmpError {
    BufferTooShort,
    OutputBufferTooSmall,
    BadChecksum,
    /// we only handle echo request/reply
    NotEcho,
}

/// a parsed ICMP echo request or reply
pub struct IcmpEcho<'a> {
    pub msg_type: u8,
    pub id: u16,
    pub seq: u16,
    pub data: &'a [u8],
}

impl<'a> IcmpEcho<'a> {
    pub fn parse(buf: &'a [u8]) -> Result<Self, IcmpError> {
        if buf.len() < HEADER_LEN {
            return Err(IcmpError::BufferTooShort);
        }

        let msg_type = buf[0];
        if msg_type != TYPE_ECHO_REQUEST && msg_type != TYPE_ECHO_REPLY {
            return Err(IcmpError::NotEcho);
        }

        // checksum covers the whole ICMP message
        if checksum(buf) != 0 {
            return Err(IcmpError::BadChecksum);
        }

        let id = u16::from_be_bytes([buf[4], buf[5]]);
        let seq = u16::from_be_bytes([buf[6], buf[7]]);

        Ok(IcmpEcho {
            msg_type,
            id,
            seq,
            data: &buf[HEADER_LEN..],
        })
    }

    pub fn is_request(&self) -> bool {
        self.msg_type == TYPE_ECHO_REQUEST
    }
}

/// build an echo reply for an incoming echo request
/// copies the request's id, seq, and data — returns bytes written
pub fn build_echo_reply(request: &IcmpEcho, out: &mut [u8]) -> Result<usize, IcmpError> {
    let total = HEADER_LEN + request.data.len();
    if out.len() < total {
        return Err(IcmpError::OutputBufferTooSmall);
    }

    out[0] = TYPE_ECHO_REPLY;
    out[1] = 0; // code
    out[2..4].copy_from_slice(&[0x00, 0x00]); // checksum placeholder
    out[4..6].copy_from_slice(&request.id.to_be_bytes());
    out[6..8].copy_from_slice(&request.seq.to_be_bytes());
    out[HEADER_LEN..total].copy_from_slice(request.data);

    let cs = checksum(&out[..total]);
    out[2..4].copy_from_slice(&cs.to_be_bytes());

    Ok(total)
}
