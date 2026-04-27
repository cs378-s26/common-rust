use alloc::sync::Arc;

use crate::{
    fs::file::{File, FileError},
    net::{
        Ipv4Addr,
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
