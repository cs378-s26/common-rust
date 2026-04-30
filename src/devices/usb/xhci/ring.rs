use core::{
    ptr,
    sync::atomic::{Ordering, fence},
};

use crate::{
    devices::usb::xhci::registers::{TRB_CYCLE, TRB_TC, TRB_TYPE_LINK, trb_type},
    memory::dma::DmaRegion,
};

pub const TRB_BYTES: usize = 16;
pub const TRBS_PER_RING: usize = 256;
pub const TRB_RING_BYTES: usize = TRB_BYTES * TRBS_PER_RING;

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct Trb {
    pub data: [u32; 4],
}

impl Trb {
    pub const fn zero() -> Self {
        Self { data: [0; 4] }
    }
}

/// Command ring or per-endpoint Transfer ring. The last slot is reserved for
/// a Link TRB back to slot 0 with the Toggle Cycle bit set; we never enqueue
/// a non-Link TRB at the last slot.
pub struct ProducerRing {
    pub virt_addr: usize,
    pub phys_addr: usize,
    pub enqueue_idx: usize,
    pub cycle: bool,
}

impl ProducerRing {
    pub fn new(dma: &DmaRegion) -> Self {
        assert!(dma.size() >= TRB_RING_BYTES);
        let virt_addr = dma.virt_addr();
        let phys_addr = dma.phys_addr();

        let link_idx = TRBS_PER_RING - 1;
        let link_slot = (virt_addr + link_idx * TRB_BYTES) as *mut u32;
        // SAFETY: DmaRegion is mapped writable and aligned; we own it.
        unsafe {
            ptr::write_volatile(link_slot.add(0), (phys_addr & 0xFFFF_FFFF) as u32);
            ptr::write_volatile(link_slot.add(1), (phys_addr >> 32) as u32);
            ptr::write_volatile(link_slot.add(2), 0);
            ptr::write_volatile(
                link_slot.add(3),
                trb_type(TRB_TYPE_LINK) | TRB_TC | TRB_CYCLE,
            );
        }

        Self {
            virt_addr,
            phys_addr,
            enqueue_idx: 0,
            cycle: true,
        }
    }

    /// Enqueue a TRB and return its physical address — used as the key when
    /// matching the eventual completion event back to a `Promise`. The cycle
    /// bit in `dw3_no_cycle` must be 0; this method sets it.
    pub fn enqueue(&mut self, dw0: u32, dw1: u32, dw2: u32, dw3_no_cycle: u32) -> u64 {
        if self.enqueue_idx == TRBS_PER_RING - 1 {
            self.write_link_cycle();
            self.enqueue_idx = 0;
            self.cycle = !self.cycle;
        }

        let slot_addr = self.virt_addr + self.enqueue_idx * TRB_BYTES;
        let slot_phys = self.phys_addr + self.enqueue_idx * TRB_BYTES;
        let slot = slot_addr as *mut u32;

        let dw3 = if self.cycle {
            dw3_no_cycle | TRB_CYCLE
        } else {
            dw3_no_cycle & !TRB_CYCLE
        };

        // SAFETY: slot is in our DmaRegion which lives for the controller's lifetime.
        // The cycle-bit dword goes last so the HC never sees a partial TRB.
        unsafe {
            ptr::write_volatile(slot.add(0), dw0);
            ptr::write_volatile(slot.add(1), dw1);
            ptr::write_volatile(slot.add(2), dw2);
            fence(Ordering::Release);
            ptr::write_volatile(slot.add(3), dw3);
        }

        self.enqueue_idx += 1;
        slot_phys as u64
    }

    fn write_link_cycle(&self) {
        let link_idx = TRBS_PER_RING - 1;
        let link_dw3_addr = (self.virt_addr + link_idx * TRB_BYTES + 12) as *mut u32;
        // SAFETY: pointer is inside our owned DMA region.
        unsafe {
            let mut dw3 = ptr::read_volatile(link_dw3_addr);
            dw3 = (dw3 & !TRB_CYCLE) | if self.cycle { TRB_CYCLE } else { 0 };
            fence(Ordering::Release);
            ptr::write_volatile(link_dw3_addr, dw3);
        }
    }
}

/// Consumer-side state for the event ring. A TRB whose cycle bit doesn't
/// match `cycle` means the ring is empty.
pub struct EventRingState {
    pub virt_addr: usize,
    pub phys_addr: usize,
    pub dequeue_idx: usize,
    pub cycle: bool,
    pub num_entries: usize,
}

impl EventRingState {
    pub fn new(dma: &DmaRegion) -> Self {
        let num_entries = dma.size() / TRB_BYTES;
        Self {
            virt_addr: dma.virt_addr(),
            phys_addr: dma.phys_addr(),
            dequeue_idx: 0,
            cycle: true,
            num_entries,
        }
    }

    pub fn peek(&self) -> Option<Trb> {
        let slot = (self.virt_addr + self.dequeue_idx * TRB_BYTES) as *const u32;
        // SAFETY: dequeue_idx is bounded by num_entries; slot is in DMA region.
        let dw3 = unsafe { ptr::read_volatile(slot.add(3)) };
        let cycle_bit = (dw3 & TRB_CYCLE) != 0;
        if cycle_bit != self.cycle {
            return None;
        }
        // DW0..DW2 are read after the cycle-bit check (with an Acquire fence)
        // so we don't observe a stale TRB body the HC is mid-write.
        fence(Ordering::Acquire);
        let dw0 = unsafe { ptr::read_volatile(slot.add(0)) };
        let dw1 = unsafe { ptr::read_volatile(slot.add(1)) };
        let dw2 = unsafe { ptr::read_volatile(slot.add(2)) };
        Some(Trb {
            data: [dw0, dw1, dw2, dw3],
        })
    }

    pub fn advance(&mut self) {
        self.dequeue_idx += 1;
        if self.dequeue_idx >= self.num_entries {
            self.dequeue_idx = 0;
            self.cycle = !self.cycle;
        }
    }

    pub fn current_phys(&self) -> u64 {
        (self.phys_addr + self.dequeue_idx * TRB_BYTES) as u64
    }
}

pub fn build_setup_stage(
    bm_request_type: u8,
    b_request: u8,
    w_value: u16,
    w_index: u16,
    w_length: u16,
    trt: u32,
) -> (u32, u32, u32, u32) {
    use crate::devices::usb::xhci::registers::{TRB_IDT, TRB_TYPE_SETUP_STAGE};
    let dw0 = (bm_request_type as u32) | ((b_request as u32) << 8) | ((w_value as u32) << 16);
    let dw1 = (w_index as u32) | ((w_length as u32) << 16);
    let dw2 = 8u32;
    let dw3 = trb_type(TRB_TYPE_SETUP_STAGE) | TRB_IDT | trt;
    (dw0, dw1, dw2, dw3)
}

pub fn build_data_stage(
    buffer_phys: u64,
    transfer_length: u32,
    dir_in: bool,
) -> (u32, u32, u32, u32) {
    use crate::devices::usb::xhci::registers::TRB_TYPE_DATA_STAGE;
    let dw0 = (buffer_phys & 0xFFFF_FFFF) as u32;
    let dw1 = (buffer_phys >> 32) as u32;
    let dw2 = transfer_length & 0x1FFFF;
    let mut dw3 = trb_type(TRB_TYPE_DATA_STAGE);
    if dir_in {
        dw3 |= 1 << 16;
    }
    (dw0, dw1, dw2, dw3)
}

pub fn build_status_stage(dir_in: bool, ioc: bool) -> (u32, u32, u32, u32) {
    use crate::devices::usb::xhci::registers::{TRB_IOC, TRB_TYPE_STATUS_STAGE};
    let mut dw3 = trb_type(TRB_TYPE_STATUS_STAGE);
    if dir_in {
        dw3 |= 1 << 16;
    }
    if ioc {
        dw3 |= TRB_IOC;
    }
    (0, 0, 0, dw3)
}

pub fn build_enable_slot() -> (u32, u32, u32, u32) {
    use crate::devices::usb::xhci::registers::TRB_TYPE_ENABLE_SLOT;
    (0, 0, 0, trb_type(TRB_TYPE_ENABLE_SLOT))
}

pub fn build_address_device(input_ctx_phys: u64, slot_id: u8, bsr: bool) -> (u32, u32, u32, u32) {
    use crate::devices::usb::xhci::registers::{TRB_BSR, TRB_TYPE_ADDRESS_DEVICE, trb_slot_id};
    let dw0 = (input_ctx_phys & 0xFFFF_FFFF) as u32;
    let dw1 = (input_ctx_phys >> 32) as u32;
    let dw2 = 0;
    let mut dw3 = trb_type(TRB_TYPE_ADDRESS_DEVICE) | trb_slot_id(slot_id);
    if bsr {
        dw3 |= TRB_BSR;
    }
    (dw0, dw1, dw2, dw3)
}

pub fn build_configure_endpoint(input_ctx_phys: u64, slot_id: u8) -> (u32, u32, u32, u32) {
    use crate::devices::usb::xhci::registers::{TRB_TYPE_CONFIGURE_ENDPOINT, trb_slot_id};
    let dw0 = (input_ctx_phys & 0xFFFF_FFFF) as u32;
    let dw1 = (input_ctx_phys >> 32) as u32;
    (
        dw0,
        dw1,
        0,
        trb_type(TRB_TYPE_CONFIGURE_ENDPOINT) | trb_slot_id(slot_id),
    )
}
