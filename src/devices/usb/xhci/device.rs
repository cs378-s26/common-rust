//! Per-USB-device state captured during enumeration.

use crate::{
    devices::usb::xhci::{
        context::ContextBlob,
        descriptors::{DeviceDescriptor, ParsedConfig},
        ring::ProducerRing,
    },
    memory::dma::DmaRegion,
    sync::IntMutex,
};

/// Resources owned per slot. The DMA buffers and contexts must outlive the
/// controller — the HC keeps reading them through the DCBAA.
pub struct UsbDevice {
    pub slot_id: u8,
    pub port: u8,
    pub speed: u32,
    pub address: u8,
    pub device_descriptor: Option<DeviceDescriptor>,
    pub config: Option<ParsedConfig>,

    pub input_ctx: ContextBlob,
    pub device_ctx: ContextBlob,
    pub ep0_tr_dma: DmaRegion,
    pub ep0_tr: IntMutex<ProducerRing>,
}
