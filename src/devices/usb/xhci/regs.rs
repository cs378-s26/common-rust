// xHCI register offsets and bit definitions

pub const CAP_CAPLENGTH: usize = 0x00; // u8  — byte length of cap register block
pub const CAP_HCSPARAMS1: usize = 0x04; // u32
pub const CAP_DBOFF: usize = 0x14; // u32 — doorbell array offset from BAR0
pub const CAP_RTSOFF: usize = 0x18; // u32 — runtime register set offset from BAR0

// HCSPARAMS1 fields
pub const HCSPARAMS1_MAX_SLOTS_MASK: u32 = 0xFF;
pub const HCSPARAMS1_MAX_PORTS_SHIFT: u32 = 24;
pub const HCSPARAMS1_MAX_PORTS_MASK: u32 = 0xFF << 24;

pub const OP_USBCMD: usize = 0x00; // u32
pub const OP_USBSTS: usize = 0x04; // u32
pub const OP_CRCR: usize = 0x18; // u64 — command ring control register
pub const OP_DCBAAP: usize = 0x30; // u64 — device context base address array pointer
pub const OP_CONFIG: usize = 0x38; // u32 — max device slots enabled

// Port registers base (each port occupies 0x10 bytes)
pub const PORT_BASE: usize = 0x400;
pub const PORT_STRIDE: usize = 0x10;
pub const PORT_PORTSC: usize = 0x00; // PORTSC offset within one port's register block

// USBCMD bits
pub const USBCMD_RUN: u32 = 1 << 0; // Run/Stop
pub const USBCMD_HCRST: u32 = 1 << 1; // Host Controller Reset
pub const USBCMD_INTE: u32 = 1 << 2; // Interrupter Enable
pub const USBCMD_HSEE: u32 = 1 << 3; // Host System Error Enable

// USBSTS bits
pub const USBSTS_HCH: u32 = 1 << 0; // HCHalted (read-only)
pub const USBSTS_CNR: u32 = 1 << 11; // Controller Not Ready (read-only)

// PORTSC bits (some are write-1-to-clear — be careful on write)
pub const PORTSC_CCS: u32 = 1 << 0; // Current Connect Status
pub const PORTSC_PED: u32 = 1 << 1; // Port Enabled/Disabled
pub const PORTSC_PR: u32 = 1 << 4; // Port Reset (initiate reset)
pub const PORTSC_PP: u32 = 1 << 9; // Port Power
pub const PORTSC_SPEED_SHIFT: u32 = 10;
pub const PORTSC_SPEED_MASK: u32 = 0xF << 10;
pub const PORTSC_CSC: u32 = 1 << 17; // Connect Status Change (w1c)
pub const PORTSC_PEC: u32 = 1 << 18; // Port Enable/Disable Change (w1c)
pub const PORTSC_PRC: u32 = 1 << 21; // Port Reset Change (w1c)
pub const PORTSC_WRC: u32 = 1 << 22; // Warm Port Reset Change (w1c)
pub const PORTSC_OCC: u32 = 1 << 23; // Over-Current Change (w1c)
// Mask of all w1c bits — must be preserved when writing PORTSC for other purposes
pub const PORTSC_W1C_MASK: u32 = PORTSC_CSC | PORTSC_PEC | PORTSC_PRC | PORTSC_WRC | PORTSC_OCC;

// PORTSC speed encoding
pub const SPEED_LOW: u32 = 2; // USB 1.1 Low Speed (1.5 Mbps)
pub const SPEED_HIGH: u32 = 3; // USB 2.0 High Speed (480 Mbps)
pub const SPEED_SUPER: u32 = 4; // USB 3.0 Super Speed (5 Gbps)
pub const SPEED_SUPER_PLUS: u32 = 5; // USB 3.1 Super Speed+ (10 Gbps)

//  Runtime registers (relative to BAR0 + RTSOFF)
pub const RT_IR0: usize = 0x020; // start of interrupter 0 register set

// Interrupter register offsets (relative to IR0 base)
pub const IR_IMAN: usize = 0x00; // u32 — interrupt management
pub const IR_ERSTSZ: usize = 0x08; // u32 — event ring segment table size
pub const IR_ERSTBA: usize = 0x10; // u64 — event ring segment table base address
pub const IR_ERDP: usize = 0x18; // u64 — event ring dequeue pointer

// IMAN bits
pub const IMAN_IP: u32 = 1 << 0; // Interrupt Pending (write 1 to clear)
pub const IMAN_IE: u32 = 1 << 1; // Interrupt Enable

pub const ERDP_EHB: u64 = 1 << 3; // Event Handler Busy (write 1 to clear)

// PCI capability ID
pub const PCI_CAP_MSIX: u8 = 0x11;

pub const MSI_ADDR_LAPIC0: u32 = 0xFEE0_0000;
// Interrupt vector allocated for xHCI MSI
pub const XHCI_MSI_VECTOR: u8 = 0x30;
pub const MSI_DATA_VALUE: u16 = XHCI_MSI_VECTOR as u16;
