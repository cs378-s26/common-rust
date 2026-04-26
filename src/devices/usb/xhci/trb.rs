use core::sync::atomic::{Ordering, fence};

/// Transfer Request Block — the fundamental unit of xHCI communication.
/// Every TRB is exactly 16 bytes and must be 16-byte aligned.
#[repr(C, align(16))]
#[derive(Clone, Copy, Default)]
pub struct Trb {
    pub param: u64,
    pub status: u32,
    pub control: u32,
}

// TRB type values (placed in control bits [15:10])
pub mod trb_type {
    pub const NORMAL: u32 = 1;
    pub const SETUP_STAGE: u32 = 2;
    pub const DATA_STAGE: u32 = 3;
    pub const STATUS_STAGE: u32 = 4;
    pub const LINK: u32 = 6;
    pub const ENABLE_SLOT: u32 = 9;
    pub const ADDRESS_DEVICE: u32 = 11;
    pub const CONFIGURE_EP: u32 = 12;
    pub const TRANSFER_EVENT: u32 = 32;
    pub const CMD_COMPLETION: u32 = 33;
    pub const PORT_STATUS_CHANGE: u32 = 34;
}

pub const TRB_TYPE_SHIFT: u32 = 10;
pub const TRB_TYPE_MASK: u32 = 0x3F << TRB_TYPE_SHIFT;

// Completion codes (from status bits [31:24] of event TRBs)
pub mod cc {
    pub const SUCCESS: u8 = 1;
    pub const SHORT_PACKET: u8 = 13;
}

// Control transfer direction
pub const TRB_DIR_IN: u32 = 1 << 16;

impl Trb {
    #[inline]
    pub fn trb_type(&self) -> u32 {
        (self.control >> TRB_TYPE_SHIFT) & 0x3F
    }

    #[inline]
    pub fn cycle(&self) -> bool {
        self.control & 1 != 0
    }

    /// Completion code from a CCE or Transfer Event TRB (status bits [31:24]).
    #[inline]
    pub fn completion_code(&self) -> u8 {
        ((self.status >> 24) & 0xFF) as u8
    }

    /// Slot ID from a CCE or Transfer Event TRB (control bits [31:24]).
    #[inline]
    pub fn slot_id(&self) -> u8 {
        ((self.control >> 24) & 0xFF) as u8
    }

    /// 1-based port number from a Port Status Change Event TRB (param bits [31:24]).
    #[inline]
    pub fn port_id(&self) -> u8 {
        ((self.param >> 24) & 0xFF) as u8
    }

    /// Endpoint ID from a Transfer Event TRB (control bits [20:16]).
    #[inline]
    pub fn endpoint_id(&self) -> u8 {
        ((self.control >> 16) & 0x1F) as u8
    }

    /// Residual transfer length from a Transfer Event TRB (status bits [23:0]).
    #[inline]
    pub fn transfer_length_remaining(&self) -> u32 {
        self.status & 0x00FF_FFFF
    }

    /// Link TRB: wraps the ring back to `ring_phys`.
    /// `toggle_cycle` causes the cycle bit to flip at this point.
    pub fn link(ring_phys: u64, toggle_cycle: bool, cycle: bool) -> Trb {
        let tc = if toggle_cycle { 1 << 1 } else { 0 };
        Trb {
            param: ring_phys,
            status: 0,
            control: (trb_type::LINK << TRB_TYPE_SHIFT) | tc | (cycle as u32),
        }
    }

    /// Enable Slot command.
    pub fn enable_slot(cycle: bool) -> Trb {
        Trb {
            param: 0,
            status: 0,
            control: (trb_type::ENABLE_SLOT << TRB_TYPE_SHIFT) | (cycle as u32),
        }
    }

    /// Address Device command.
    pub fn address_device(input_ctx_phys: u64, slot_id: u8, cycle: bool) -> Trb {
        Trb {
            param: input_ctx_phys,
            status: 0,
            control: (trb_type::ADDRESS_DEVICE << TRB_TYPE_SHIFT)
                | ((slot_id as u32) << 24)
                | (cycle as u32),
        }
    }

    /// Configure Endpoint command.
    pub fn configure_endpoint(input_ctx_phys: u64, slot_id: u8, cycle: bool) -> Trb {
        Trb {
            param: input_ctx_phys,
            status: 0,
            control: (trb_type::CONFIGURE_EP << TRB_TYPE_SHIFT)
                | ((slot_id as u32) << 24)
                | (cycle as u32),
        }
    }

    /// Normal TRB for bulk/interrupt data transfers (always sets IOC).
    pub fn normal(data_phys: u64, len: u32, cycle: bool) -> Trb {
        Trb {
            param: data_phys,
            status: len & 0x0001_FFFF, // transfer length in bits [16:0]
            control: (trb_type::NORMAL << TRB_TYPE_SHIFT) | (1 << 5) | (cycle as u32),
        }
    }

    /// Setup Stage TRB for control transfers (8-byte setup packet inline).
    /// TRB Transfer Length must always be 8; TRT=3 (IN data stage).
    pub fn setup_stage(setup_packet: u64, cycle: bool) -> Trb {
        Trb {
            param: setup_packet,
            status: 8,
            control: (trb_type::SETUP_STAGE << TRB_TYPE_SHIFT)
                | (3 << 16) // TRT = IN data stage
                | (1 << 6)  // IDT = immediate data
                | (cycle as u32),
        }
    }

    /// Data Stage TRB for control IN transfers.
    /// IOC is intentionally omitted here — only the Status Stage generates a
    /// Transfer Event.  This prevents a stale Data Stage event from being
    /// misidentified as the next transfer's completion.
    pub fn data_stage_in(data_phys: u64, len: u16, cycle: bool) -> Trb {
        Trb {
            param: data_phys,
            status: len as u32,
            control: (trb_type::DATA_STAGE << TRB_TYPE_SHIFT)
                | TRB_DIR_IN // direction IN
                // NO IOC — wait_for_transfer fires on Status Stage only
                | (cycle as u32),
        }
    }

    /// Status Stage TRB for control IN transfers (status phase is OUT).
    pub fn status_stage_out(cycle: bool) -> Trb {
        Trb {
            param: 0,
            status: 0,
            control: (trb_type::STATUS_STAGE << TRB_TYPE_SHIFT)
                | (1 << 5) // IOC
                | (cycle as u32),
            // direction bit = 0 (OUT) for status after IN data
        }
    }

    /// Status Stage TRB for control OUT transfers (status phase is IN).
    pub fn status_stage_in(cycle: bool) -> Trb {
        Trb {
            param: 0,
            status: 0,
            control: (trb_type::STATUS_STAGE << TRB_TYPE_SHIFT)
                | TRB_DIR_IN
                | (1 << 5)  // IOC
                | (cycle as u32),
        }
    }

    /// # Safety
    /// `dst` must be valid, 16-byte aligned, and backed by DMA memory visible to the device.
    pub unsafe fn write_volatile_to(self, dst: *mut Trb) {
        unsafe {
            core::ptr::write_volatile(&raw mut (*dst).param, self.param);
            core::ptr::write_volatile(&raw mut (*dst).status, self.status);
            fence(Ordering::Release);
            core::ptr::write_volatile(&raw mut (*dst).control, self.control);
        }
    }

    /// # Safety
    /// `src` must be valid and 16-byte aligned.
    pub unsafe fn read_volatile_from(src: *const Trb) -> Trb {
        unsafe {
            let param = core::ptr::read_volatile(&raw const (*src).param);
            let status = core::ptr::read_volatile(&raw const (*src).status);
            let control = core::ptr::read_volatile(&raw const (*src).control);
            Trb {
                param,
                status,
                control,
            }
        }
    }
}
