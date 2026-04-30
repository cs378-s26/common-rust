use core::ptr;

use crate::{devices::usb::xhci::descriptors::EndpointDescriptor, memory::dma::DmaRegion};

pub const CTX_SIZE_32: usize = 32;
pub const CTX_SIZE_64: usize = 64;
// Device Context Base Address Array 
pub struct Dcbaa {
    pub dma: DmaRegion,
}

impl Default for Dcbaa {
    fn default() -> Self {
        Self::new()
    }
}

impl Dcbaa {
    pub fn new() -> Self {
        Self {
            dma: DmaRegion::new(1),
        }
    }

    pub fn set(&self, slot: u8, phys: u64) {
        let p = (self.dma.virt_addr() + (slot as usize) * 8) as *mut u64;
        // SAFETY: slot < 256, so offset is < 2048; DMA region is 4096 bytes.
        unsafe {
            ptr::write_volatile(p, phys);
        }
    }

    pub fn phys_addr(&self) -> u64 {
        self.dma.phys_addr() as u64
    }
}

#[derive(Clone, Copy)]
pub enum EpType {
    IsochOut = 1,
    BulkOut = 2,
    InterruptOut = 3,
    Control = 4,
    IsochIn = 5,
    BulkIn = 6,
    InterruptIn = 7,
}

impl EpType {
    pub fn from_descriptor(desc: &EndpointDescriptor) -> Self {
        let in_dir = desc.is_in();
        match (desc.transfer_type(), in_dir) {
            (0, _) => Self::Control,
            (1, false) => Self::IsochOut,
            (1, true) => Self::IsochIn,
            (2, false) => Self::BulkOut,
            (2, true) => Self::BulkIn,
            (3, false) => Self::InterruptOut,
            (3, true) => Self::InterruptIn,
            _ => Self::BulkIn,
        }
    }
}

pub struct ContextBlob {
    pub dma: DmaRegion,
    pub ctx_size: usize,
}

impl ContextBlob {
    pub fn new(num_ctx_entries: usize, ctx_size: usize) -> Self {
        let bytes = num_ctx_entries * ctx_size;
        Self {
            dma: DmaRegion::new_bytes(bytes),
            ctx_size,
        }
    }

    pub fn phys_addr(&self) -> u64 {
        self.dma.phys_addr() as u64
    }

    pub fn zero(&self) {
        self.dma.zero();
    }

    fn ctx_ptr(&self, idx: usize) -> *mut u32 {
        (self.dma.virt_addr() + idx * self.ctx_size) as *mut u32
    }

    pub fn write_dword(&self, ctx_idx: usize, dword_idx: usize, value: u32) {
        let p = unsafe { self.ctx_ptr(ctx_idx).add(dword_idx) };
        unsafe { ptr::write_volatile(p, value) };
    }

    pub fn read_dword(&self, ctx_idx: usize, dword_idx: usize) -> u32 {
        let p = unsafe { self.ctx_ptr(ctx_idx).add(dword_idx) };
        unsafe { ptr::read_volatile(p) }
    }
}

pub fn slot_context_dw0(route_string: u32, speed: u32, context_entries: u32) -> u32 {
    (route_string & 0xFFFFF) | ((speed & 0xF) << 20) | ((context_entries & 0x1F) << 27)
}

pub fn slot_context_dw1(port_number: u8) -> u32 {
    (port_number as u32) << 16
}

/// EP0 (Control) context dwords per xHCI  6.2.3.
pub fn ep0_context_dwords(
    max_packet_size0: u32,
    tr_dequeue_phys: u64,
    cycle_bit: bool,
) -> [u32; 5] {
    let dw0 = 0u32;
    // DW1: CErr=3, EP Type=Control(4), MaxPacketSize.
    let dw1 = (3 << 1) | (4 << 3) | (max_packet_size0 << 16);
    let dw2 = (tr_dequeue_phys & 0xFFFF_FFF0) as u32 | if cycle_bit { 1 } else { 0 };
    let dw3 = (tr_dequeue_phys >> 32) as u32;
    let dw4 = 8u32;
    [dw0, dw1, dw2, dw3, dw4]
}

pub fn endpoint_context_dwords(
    desc: &EndpointDescriptor,
    speed: u32,
    tr_dequeue_phys: u64,
    cycle_bit: bool,
) -> [u32; 5] {
    let ep_type = EpType::from_descriptor(desc) as u32;
    let max_packet_size = desc.w_max_packet_size as u32 & 0x7FF;

    // Interval encoding per xHCI 6.2.3.6 — depends on speed AND EP type.
    let interval = match (speed, desc.transfer_type()) {
        (super::registers::PORT_SPEED_FULL | super::registers::PORT_SPEED_LOW, 3) => {
            log2_floor(desc.b_interval as u32).saturating_add(3) as u32
        }
        (_, 3) | (_, 1) => (desc.b_interval as u32).saturating_sub(1),
        _ => 0,
    };

    let dw0 = (interval & 0xFF) << 16;
    let dw1 = (3 << 1) | (ep_type << 3) | (max_packet_size << 16);
    let dw2 = (tr_dequeue_phys & 0xFFFF_FFF0) as u32 | if cycle_bit { 1 } else { 0 };
    let dw3 = (tr_dequeue_phys >> 32) as u32;
    // Per xHCI  6.2.3.5, Max ESIT Payload Lo (DW4 bits[31:16]) is
    // wMaxPacketSize * (1 + MaxBurstSize) for Iso/Int — required to be > 0.
    // We don't enable HS high-bandwidth, so MaxBurstSize=0.
    let max_esit_payload_lo = match desc.transfer_type() {
        1 | 3 => max_packet_size,
        _ => 0,
    };
    let dw4 = max_packet_size | (max_esit_payload_lo << 16);
    [dw0, dw1, dw2, dw3, dw4]
}

fn log2_floor(x: u32) -> u8 {
    if x == 0 {
        return 0;
    }
    (31 - x.leading_zeros()) as u8
}
