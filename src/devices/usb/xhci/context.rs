use crate::memory::dma::DmaRegion;

const CTX_SIZE: usize = 32;

// Slot Context (xHCI spec 6.2.2)
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct SlotContext {
    pub dw0: u32, // RouteString[19:0], Speed[23:20], MTT[25], Hub[26], ContextEntries[31:27]
    pub dw1: u32, // MaxExitLatency[15:0], RootHubPortNum[23:16], NumPorts[31:24]
    pub dw2: u32, // ParentHubSlot[7:0], ParentPortNum[15:8], TTT[17:16], IRQTarget[31:22]
    pub dw3: u32, // DeviceAddress[7:0], SlotState[31:27]
    pub rsvd: [u32; 4],
}

impl SlotContext {
    pub fn set_speed(&mut self, speed: u32) {
        self.dw0 = (self.dw0 & !(0xF << 20)) | ((speed & 0xF) << 20);
    }

    /// 1-based root hub port number.
    pub fn set_root_hub_port(&mut self, port: u8) {
        self.dw1 = (self.dw1 & !0x00FF_0000) | ((port as u32) << 16);
    }

    /// Number of valid endpoint context entries in the device context (1..=31).
    pub fn set_context_entries(&mut self, entries: u8) {
        self.dw0 = (self.dw0 & !(0x1F << 27)) | ((entries as u32 & 0x1F) << 27);
    }
}

// Endpoint Context (xHCI spec 6.2.3)
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct EndpointContext {
    pub dw0: u32, // EPState[2:0], Mult[9:8], MaxPStreams[14:10], Interval[23:16]
    pub dw1: u32, // CErr[2:1], EPType[5:3], HID[7], MaxBurstSize[15:8], MaxPacketSize[31:16]
    pub dw2: u32, // DCS[0], TRDequeuePointer_Lo[31:4]
    pub dw3: u32, // TRDequeuePointer_Hi
    pub dw4: u32, // AvgTRBLength[15:0], MaxESITPayloadLo[31:16]
    pub rsvd: [u32; 3],
}

pub mod ep_type {
    pub const CONTROL: u32 = 4;
    pub const INTERRUPT_IN: u32 = 7;
}

impl EndpointContext {
    pub fn set_ep_type(&mut self, ty: u32) {
        self.dw1 = (self.dw1 & !(0x7 << 3)) | ((ty & 0x7) << 3);
    }

    pub fn set_max_packet_size(&mut self, mps: u16) {
        self.dw1 = (self.dw1 & 0x0000_FFFF) | ((mps as u32) << 16);
    }

    /// CErr: error count (2 = retry twice before generating transfer event).
    pub fn set_cerr(&mut self, cerr: u8) {
        self.dw1 = (self.dw1 & !(0x3 << 1)) | ((cerr as u32 & 0x3) << 1);
    }

    /// Set Transfer Ring Dequeue Pointer and Dequeue Cycle State bit.
    pub fn set_tr_dequeue_ptr(&mut self, phys: u64, dcs: bool) {
        self.dw2 = (phys as u32 & 0xFFFF_FFF0) | (dcs as u32);
        self.dw3 = (phys >> 32) as u32;
    }

    pub fn set_hid(&mut self) {
        self.dw1 |= 1 << 7;
    }

    /// Polling interval (125 microseconds for HS/SS, 1 ms for FS/LS).
    pub fn set_interval(&mut self, interval: u8) {
        self.dw0 = (self.dw0 & !0x00FF_0000) | ((interval as u32) << 16);
    }

    /// Average TRB length hint (used by the controller for scheduling).
    pub fn set_avg_trb_length(&mut self, len: u16) {
        self.dw4 = (self.dw4 & 0xFFFF_0000) | (len as u32);
    }
}

// Input Control Context (xHCI spec 6.2.5)
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct InputControlContext {
    pub drop_flags: u32,
    pub add_flags: u32,
    pub rsvd: [u32; 6],
}

pub struct InputContext {
    mem: DmaRegion,
}

impl InputContext {
    /// Allocate an input context for a device with `num_ep_contexts` endpoint contexts.
    pub fn new(num_ep_contexts: usize) -> Self {
        let bytes = (2 + num_ep_contexts) * CTX_SIZE;
        Self {
            mem: DmaRegion::new_bytes(bytes),
        }
    }

    pub fn phys(&self) -> u64 {
        self.mem.phys_addr() as u64
    }

    pub fn icc_mut(&mut self) -> &mut InputControlContext {
        unsafe { &mut *(self.mem.virt_addr() as *mut InputControlContext) }
    }

    pub fn slot_mut(&mut self) -> &mut SlotContext {
        unsafe { &mut *((self.mem.virt_addr() + CTX_SIZE) as *mut SlotContext) }
    }

    /// `ep_idx` 0 = EP0 (control endpoint).
    pub fn ep_mut(&mut self, ep_idx: usize) -> &mut EndpointContext {
        let offset = (2 + ep_idx) * CTX_SIZE;
        unsafe { &mut *((self.mem.virt_addr() + offset) as *mut EndpointContext) }
    }
}

pub struct DeviceContext {
    mem: DmaRegion,
}

impl DeviceContext {
    /// Allocate a device context for `max_ep` endpoint entries plus the slot context.
    pub fn new(max_ep: usize) -> Self {
        Self {
            mem: DmaRegion::new_bytes((1 + max_ep) * CTX_SIZE),
        }
    }

    pub fn phys(&self) -> u64 {
        self.mem.phys_addr() as u64
    }
}
