use alloc::sync::Arc;

use crate::{
    fs::file::{File, FileError},
    net::{
        Ipv4Addr,
        tcp::TcpConn,
        udp::{UDP_DEMUX, UdpDatagram, UdpSink},
    },
    sync::{BoundedBuffer, IntMutex, MutexLike},
};

const RX_QUEUE_DEPTH: usize = 32;

pub struct UdpSocket {
    local_addr: IntMutex<Option<(Ipv4Addr, u16)>>,
    rx_queue: BoundedBuffer<UdpDatagram, RX_QUEUE_DEPTH>,
}

impl UdpSocket {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            local_addr: IntMutex::new(None),
            rx_queue: BoundedBuffer::new(),
        })
    }

    /// Record the local address. Caller (sys_bind) must also call UDP_DEMUX.register.
    /// A socket may only be bound once.
    pub fn bind(&self, ip: Ipv4Addr, port: u16) {
        *self.local_addr.lock() = Some((ip, port));
    }

    pub fn local_addr(&self) -> Option<(Ipv4Addr, u16)> {
        *self.local_addr.lock()
    }

    /// Pop the next received datagram, blocking until one arrives.
    /// Used by recvfrom to get src_ip/src_port alongside the payload.
    pub fn recv_datagram(&self) -> UdpDatagram {
        self.rx_queue.pop()
    }
}

impl UdpSink for UdpSocket {
    fn receive(&self, datagram: UdpDatagram) {
        // Drop silently if full rather than stalling the receive loop.
        self.rx_queue.try_push(datagram);
    }
}

pub struct TcpSocket {
    pub conn: Arc<TcpConn>,
}

impl TcpSocket {
    pub fn new(conn: Arc<TcpConn>) -> Arc<Self> {
        Arc::new(Self { conn })
    }
}

impl File for TcpSocket {
    fn read(&self, buf: &mut [u8]) -> Result<usize, FileError> {
        let data = self.conn.rx_buf.pop();
        let n = data.len().min(buf.len());
        buf[..n].copy_from_slice(&data[..n]);
        Ok(n)
    }

    fn write(&self, payload: &[u8]) -> Result<usize, FileError> {
        use crate::net::tcp::{FLAG_ACK, FLAG_PSH, build_tcp_segment};
        use crate::net::ethernet::{EtherType, build_ethernet_frame};
        use crate::net::ipv4::{Protocol, build_ipv4_packet};
        use crate::devices::discovery::NETWORK_DEVICES;
        use crate::sync::MutexLike as _;

        let (local_addr, remote_addr, remote_mac, local_port, remote_port, seq, ack, mss) = {
            let mut inner = self.conn.inner.lock();
            let seq = inner.snd_nxt;
            inner.snd_nxt = inner.snd_nxt.wrapping_add(payload.len() as u32);
            (inner.local_addr, inner.remote_addr, inner.remote_mac,
             inner.local_port, inner.remote_port, seq, inner.rcv_nxt, inner.mss)
        };

        let max_payload = mss as usize;
        let mut offset = 0;
        while offset < payload.len() {
            let end = (offset + max_payload).min(payload.len());
            let chunk = &payload[offset..end];

            let mut tcp_buf  = [0u8; 1536];
            let mut ip_buf   = [0u8; 1536];
            let mut frame_buf = [0u8; 1536];

            let chunk_seq = seq.wrapping_add(offset as u32);
            let Ok(tcp_len) = build_tcp_segment(
                local_addr, remote_addr, local_port, remote_port,
                chunk_seq, ack, FLAG_PSH | FLAG_ACK, 65535, None, chunk, &mut tcp_buf,
            ) else { return Err(FileError::NotSupported); };
            let Ok(ip_len) = build_ipv4_packet(
                local_addr, remote_addr, Protocol::Tcp, &tcp_buf[..tcp_len], &mut ip_buf,
            ) else { return Err(FileError::NotSupported); };
            let our_mac = {
                let devs = NETWORK_DEVICES.lock();
                devs.first().map(|n| n.mac_address()).ok_or(FileError::NotSupported)?
            };
            let Ok(frame_len) = build_ethernet_frame(
                remote_mac, our_mac, EtherType::Ipv4, &ip_buf[..ip_len], &mut frame_buf,
            ) else { return Err(FileError::NotSupported); };

            let mut devs = NETWORK_DEVICES.lock();
            if let Some(nic) = devs.first_mut() {
                let _ = nic.send_packet(&frame_buf[..frame_len]);
            }

            offset = end;
        }

        Ok(payload.len())
    }

    fn close(&self) -> Result<(), FileError> {
        self.conn.close();
        Ok(())
    }

    fn as_any_arc(self: Arc<Self>) -> Arc<dyn core::any::Any + Send + Sync> {
        self
    }
}

impl File for UdpSocket {
    fn read(&self, buf: &mut [u8]) -> Result<usize, FileError> {
        let datagram = self.rx_queue.pop();
        let n = datagram.data.len().min(buf.len());
        buf[..n].copy_from_slice(&datagram.data[..n]);
        Ok(n)
    }

    fn write(&self, _buf: &[u8]) -> Result<usize, FileError> {
        // UDP sends require a destination address — use sendto, not write.
        Err(FileError::NotSupported)
    }

    fn close(&self) -> Result<(), FileError> {
        if let Some((_, port)) = *self.local_addr.lock() {
            UDP_DEMUX.unregister(port);
        }
        Ok(())
    }

    fn as_any_arc(self: Arc<Self>) -> Arc<dyn core::any::Any + Send + Sync> {
        self
    }
}
