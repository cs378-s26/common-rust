use super::trb::Trb;
use crate::memory::dma::DmaRegion;

const TRB_SIZE: usize = core::mem::size_of::<Trb>();

/// A DMA-backed ring of TRBs.
///
/// Producer rings (command/transfer): software writes TRBs, controller consumes them.
/// The last slot is always a Link TRB that wraps back to slot 0.
///
/// Event ring: controller writes TRBs, software reads them. No Link TRB needed;
/// the ERST handles wrap-around.
pub struct Ring {
    mem: DmaRegion,
    enqueue: usize,
    dequeue: usize,
    pub cycle: bool,
    size: usize,
}

impl Ring {
    /// Create a command or transfer ring. `trb_count` includes the Link TRB slot.
    pub fn new_producer(trb_count: usize) -> Ring {
        assert!(trb_count >= 2, "ring must have at least 2 slots");
        let mem = DmaRegion::new_bytes(trb_count * TRB_SIZE);
        let mut ring = Ring {
            mem,
            enqueue: 0,
            dequeue: 0,
            cycle: true,
            size: trb_count,
        };
        let phys = ring.mem.phys_addr() as u64;
        let link = Trb::link(phys, true, ring.cycle);
        unsafe { link.write_volatile_to(ring.slot_ptr_mut(trb_count - 1)) };
        ring
    }

    /// Create an event ring (device producer, software consumer).
    pub fn new_event(trb_count: usize) -> Ring {
        Ring {
            mem: DmaRegion::new_bytes(trb_count * TRB_SIZE),
            enqueue: 0,
            dequeue: 0,
            cycle: true,
            size: trb_count,
        }
    }

    /// Physical base address of the ring (for CRCR / ERSTBA / TR dequeue).
    pub fn phys_base(&self) -> u64 {
        self.mem.phys_addr() as u64
    }

    /// Physical address of the current dequeue pointer (for ERDP updates).
    pub fn dequeue_phys(&self) -> u64 {
        self.mem.phys_addr() as u64 + (self.dequeue * TRB_SIZE) as u64
    }

    /// Push one TRB onto a producer ring, stamping the current cycle bit.
    pub fn push(&mut self, mut trb: Trb) {
        assert!(
            self.enqueue < self.size - 1,
            "ring is full (enqueue would overwrite Link TRB)"
        );

        if self.cycle {
            trb.control |= 1;
        } else {
            trb.control &= !1;
        }

        unsafe { trb.write_volatile_to(self.slot_ptr_mut(self.enqueue)) };
        self.enqueue += 1;

        if self.enqueue == self.size - 1 {
            let new_link = Trb::link(self.phys_base(), true, self.cycle);
            unsafe { new_link.write_volatile_to(self.slot_ptr_mut(self.size - 1)) };
            self.cycle = !self.cycle;
            self.enqueue = 0;
        }
    }

    /// Attempt to pop one TRB from the event ring.
    /// Returns `None` when no new TRB has been posted by the controller.
    pub fn pop_event(&mut self) -> Option<Trb> {
        let trb = unsafe { Trb::read_volatile_from(self.slot_ptr(self.dequeue)) };
        if trb.cycle() != self.cycle {
            return None;
        }
        self.dequeue += 1;
        if self.dequeue == self.size {
            self.dequeue = 0;
            self.cycle = !self.cycle;
        }
        Some(trb)
    }

    fn slot_ptr(&self, idx: usize) -> *const Trb {
        (self.mem.virt_addr() + idx * TRB_SIZE) as *const Trb
    }

    fn slot_ptr_mut(&mut self, idx: usize) -> *mut Trb {
        (self.mem.virt_addr() + idx * TRB_SIZE) as *mut Trb
    }
}

/// One entry in the Event Ring Segment Table (ERST).
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct ErstEntry {
    pub base_addr: u64,
    pub segment_size: u16,
    pub reserved: [u8; 6],
}

/// Allocate a one-entry ERST for a single-segment event ring.
pub fn alloc_erst(event_ring: &Ring) -> DmaRegion {
    let erst = DmaRegion::new_bytes(core::mem::size_of::<ErstEntry>());
    let entry = ErstEntry {
        base_addr: event_ring.phys_base(),
        segment_size: event_ring.size as u16,
        reserved: [0u8; 6],
    };
    unsafe {
        core::ptr::write_volatile(erst.virt_addr() as *mut ErstEntry, entry);
    }
    erst
}
