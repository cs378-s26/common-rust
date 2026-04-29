// Register offsets, bit fields, and TRB-type constants for xHCI.
pub const CAP_VERSION_LENGTH: usize = 0x00;
pub const CAP_HCSPARAMS1: usize = 0x04;
pub const CAP_HCSPARAMS2: usize = 0x08;
pub const CAP_HCCPARAMS1: usize = 0x10;
pub const CAP_DBOFF: usize = 0x14;
pub const CAP_RTSOFF: usize = 0x18;


pub const OP_USBCMD: usize = 0x00;
pub const OP_USBSTS: usize = 0x04;
pub const OP_PAGESIZE: usize = 0x08;
pub const OP_DNCTRL: usize = 0x14;
pub const OP_CRCR_LO: usize = 0x18;
pub const OP_CRCR_HI: usize = 0x1C;
pub const OP_DCBAAP_LO: usize = 0x30;
pub const OP_DCBAAP_HI: usize = 0x34;
pub const OP_CONFIG: usize = 0x38;
pub const OP_PORT_REGS_BASE: usize = 0x400;
pub const OP_PORT_REG_STRIDE: usize = 0x10;

// USBCMD bits
pub const USBCMD_RUN: u32 = 1 << 0;
pub const USBCMD_HCRST: u32 = 1 << 1;
pub const USBCMD_INTE: u32 = 1 << 2;

// USBSTS bits
pub const USBSTS_HCH: u32 = 1 << 0;
pub const USBSTS_HSE: u32 = 1 << 2;
pub const USBSTS_EINT: u32 = 1 << 3;
pub const USBSTS_PCD: u32 = 1 << 4;
pub const USBSTS_CNR: u32 = 1 << 11;

// CRCR bits (low dword)
pub const CRCR_RCS: u64 = 1 << 0;

// PORTSC offsets (within each port reg set)
pub const PORTSC: usize = 0x00;

// PORTSC bits
pub const PORTSC_CCS: u32 = 1 << 0;
pub const PORTSC_PED: u32 = 1 << 1;
pub const PORTSC_PR: u32 = 1 << 4;
pub const PORTSC_PLS_SHIFT: u32 = 5;
pub const PORTSC_PLS_MASK: u32 = 0xF << 5;
pub const PORTSC_PP: u32 = 1 << 9;
pub const PORTSC_SPEED_SHIFT: u32 = 10;
pub const PORTSC_SPEED_MASK: u32 = 0xF << 10;
// Port-status change RW1C bits — write-1-to-clear; write 0 to leave alone.
pub const PORTSC_CSC: u32 = 1 << 17;
pub const PORTSC_PEC: u32 = 1 << 18;
pub const PORTSC_WRC: u32 = 1 << 19;
pub const PORTSC_OCC: u32 = 1 << 20;
pub const PORTSC_PRC: u32 = 1 << 21;
pub const PORTSC_PLC: u32 = 1 << 22;
pub const PORTSC_CEC: u32 = 1 << 23;
// Bits we must preserve as zero when doing a read-modify-write that includes
// the RW1C status bits (writing 1 to the corresponding R/WC bits would clear them).
pub const PORTSC_RW1C_MASK: u32 =
    PORTSC_CSC | PORTSC_PEC | PORTSC_WRC | PORTSC_OCC | PORTSC_PRC | PORTSC_PLC | PORTSC_CEC;
// PED bit is also "RW1C-ish" — writing 1 disables the port. Leave as 0 in RMW.
pub const PORTSC_PRESERVE_MASK: u32 = !(PORTSC_RW1C_MASK | PORTSC_PED);

// Port speed values (PORTSC bits[13:10])
pub const PORT_SPEED_FULL: u32 = 1; // 12 Mb/s
pub const PORT_SPEED_LOW: u32 = 2; // 1.5 Mb/s
pub const PORT_SPEED_HIGH: u32 = 3; // 480 Mb/s
pub const PORT_SPEED_SUPER: u32 = 4; // 5 Gb/s


pub const RT_IR0_BASE: usize = 0x20; // first interrupter register set
pub const RT_IR_STRIDE: usize = 0x20;

pub const IR_IMAN: usize = 0x00;
pub const IR_IMOD: usize = 0x04;
pub const IR_ERSTSZ: usize = 0x08;
pub const IR_ERSTBA_LO: usize = 0x10;
pub const IR_ERSTBA_HI: usize = 0x14;
pub const IR_ERDP_LO: usize = 0x18;
pub const IR_ERDP_HI: usize = 0x1C;

pub const IMAN_IP: u32 = 1 << 0; // RW1C: interrupt pending
pub const IMAN_IE: u32 = 1 << 1; // interrupt enable
pub const ERDP_EHB: u64 = 1 << 3; // RW1C: event handler busy


pub const DB_HOST: usize = 0; // doorbell index 0 = host controller (command ring)
pub const DB_TARGET_COMMAND: u32 = 0;


pub const XECP_ID_USB_LEGACY_SUPPORT: u8 = 1;
pub const USBLEGSUP_BIOS_OWNED: u32 = 1 << 16;
pub const USBLEGSUP_OS_OWNED: u32 = 1 << 24;
pub const USBLEGCTLSTS_OFFSET: usize = 0x04;
pub const USBLEGCTLSTS_DISABLE_SMI_AND_CLEAR: u32 = 0xE000_0000;


pub const TRB_TYPE_NORMAL: u32 = 1;
pub const TRB_TYPE_SETUP_STAGE: u32 = 2;
pub const TRB_TYPE_DATA_STAGE: u32 = 3;
pub const TRB_TYPE_STATUS_STAGE: u32 = 4;
pub const TRB_TYPE_LINK: u32 = 6;
pub const TRB_TYPE_NO_OP: u32 = 8;
pub const TRB_TYPE_ENABLE_SLOT: u32 = 9;
pub const TRB_TYPE_DISABLE_SLOT: u32 = 10;
pub const TRB_TYPE_ADDRESS_DEVICE: u32 = 11;
pub const TRB_TYPE_CONFIGURE_ENDPOINT: u32 = 12;
pub const TRB_TYPE_EVALUATE_CONTEXT: u32 = 13;
pub const TRB_TYPE_TRANSFER_EVENT: u32 = 32;
pub const TRB_TYPE_COMMAND_COMPLETION_EVENT: u32 = 33;
pub const TRB_TYPE_PORT_STATUS_CHANGE_EVENT: u32 = 34;

// DW3 common bits
pub const TRB_CYCLE: u32 = 1 << 0;
pub const TRB_ENT: u32 = 1 << 1;
pub const TRB_TC: u32 = 1 << 1; // Toggle Cycle (Link TRB only)
pub const TRB_ISP: u32 = 1 << 2;
pub const TRB_CH: u32 = 1 << 4;
pub const TRB_IOC: u32 = 1 << 5;
pub const TRB_IDT: u32 = 1 << 6;
pub const TRB_BSR: u32 = 1 << 9; // Block SET_ADDRESS Request (Address Device)

// Setup Stage Transfer Type (TRT, DW3[17:16])
pub const TRB_TRT_NO_DATA: u32 = 0 << 16;
pub const TRB_TRT_OUT: u32 = 2 << 16;
pub const TRB_TRT_IN: u32 = 3 << 16;

// Status Stage direction (DW3[16])
pub const TRB_STATUS_DIR_IN: u32 = 1 << 16;

// Completion codes (DW2[31:24] of event TRBs)
pub const COMPLETION_SUCCESS: u8 = 1;
pub const COMPLETION_SHORT_PACKET: u8 = 13;

#[inline]
pub const fn trb_type(t: u32) -> u32 {
    t << 10
}

#[inline]
pub const fn trb_slot_id(slot: u8) -> u32 {
    (slot as u32) << 24
}

#[inline]
pub const fn trb_endpoint_id(ep: u32) -> u32 {
    (ep & 0x1F) << 16
}

#[inline]
pub fn trb_get_type(dw3: u32) -> u32 {
    (dw3 >> 10) & 0x3F
}

#[inline]
pub fn trb_get_slot_id(dw3: u32) -> u8 {
    ((dw3 >> 24) & 0xFF) as u8
}

#[inline]
pub fn event_completion_code(dw2: u32) -> u8 {
    ((dw2 >> 24) & 0xFF) as u8
}


pub const USB_REQ_GET_DESCRIPTOR: u8 = 0x06;
pub const USB_REQ_SET_ADDRESS: u8 = 0x05;
pub const USB_REQ_SET_CONFIGURATION: u8 = 0x09;

pub const USB_DESC_DEVICE: u8 = 0x01;
pub const USB_DESC_CONFIGURATION: u8 = 0x02;
pub const USB_DESC_INTERFACE: u8 = 0x04;
pub const USB_DESC_ENDPOINT: u8 = 0x05;

pub const BMREQ_DEVICE_TO_HOST: u8 = 0x80;
pub const BMREQ_HOST_TO_DEVICE: u8 = 0x00;
