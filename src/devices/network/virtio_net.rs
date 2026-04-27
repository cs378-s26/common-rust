extern crate virtio_drivers;

use virtio_drivers::{
    Hal,
    device::net::{TxBuffer, VirtIONet},
    transport::Transport,
};

use super::{NetworkDevice, NetworkError};
use crate::devices::{Device, virtio::VirtioHal};
/// VirtIONetDriver wraps the virtio-drivers VirtIONet device and ties it to our kernel's HAL
/// constructed from a transport (MMIO) and used by the device framework to send and receive packets
pub struct VirtIONetDriver<H: Hal, T: Transport, const QUEUE_SIZE: usize> {
    net: VirtIONet<H, T, QUEUE_SIZE>,
}

// TODO: VERIFY THAT THIS IS THE CASE
unsafe impl<H: Hal, T: Transport, const Q: usize> Send for VirtIONetDriver<H, T, Q> {}
unsafe impl<H: Hal, T: Transport, const Q: usize> Sync for VirtIONetDriver<H, T, Q> {}

impl<T: Transport> VirtIONetDriver<VirtioHal, T, 16> {
    pub fn new(transport: T) -> Self {
        Self {
            net: VirtIONet::<VirtioHal, T, 16>::new(transport, 1536)
                .expect("failed to initialize virtio net device"),
        }
    }
}

impl<T: Transport> NetworkDevice for VirtIONetDriver<VirtioHal, T, 16> {
    fn name(&self) -> &str {
        "virtio_net"
    }

    fn send_packet(&mut self, packet: &[u8]) -> Result<(), NetworkError> {
        self.net
            .send(TxBuffer::from(packet))
            .map_err(|_| NetworkError::SendError)
    }

    fn receive_packet(&mut self, buffer: &mut [u8]) -> Result<usize, NetworkError> {
        let rx_buf = self.net.receive().map_err(|_| NetworkError::ReceiveError)?;
        let packet = rx_buf.packet();
        let len = packet.len().min(buffer.len());
        buffer[..len].copy_from_slice(&packet[..len]);
        self.net
            .recycle_rx_buffer(rx_buf)
            .map_err(|_| NetworkError::ReceiveError)?;
        Ok(len)
    }
}

impl<T: Transport> Device for VirtIONetDriver<VirtioHal, T, 16> {
    #[allow(unused_variables)]
    fn ioctl(&self, request: u64, arg1: u64, arg2: u64) -> u64 {
        0 // stub 0 = success required by Device supertrait
    }
}
