pub mod context;
pub mod device;
pub mod discovery;
pub mod event;
pub mod regs;
pub mod ring;
pub mod trb;

extern crate alloc;

use alloc::{boxed::Box, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering, fence};

use spin::Once;

use self::{
    context::DeviceContext,
    regs::*,
    ring::{Ring, alloc_erst},
    trb::{Trb, trb_type},
};
use crate::{
    devices::discovery::pcie::{PCIE, map_bar, PcieFunction},
    memory::dma::{DmaRegion, MmioRegion},
    print::kprintln,
    sync::{IntSpinLock, MutexLike},
    thread::yield_thread,
};

pub static XHCI: Once<IntSpinLock<XhciController>> = Once::new();

/// Signal from the interrupt handler: new events are waiting.
pub static XHCI_IRQ_PENDING: AtomicBool = AtomicBool::new(false);

pub struct SlotData {
    pub dev_ctx: DeviceContext,
    pub ctrl_ring: Ring,
    pub intr_ring: Option<Ring>,
    pub hid_buf: Option<DmaRegion>,
    pub port: u8,
    pub speed: u32,
    pub intr_ep_id: u8,
    pub intr_max_pkt: u16,
}

impl SlotData {
    fn new(port: u8, speed: u32) -> Self {
        SlotData {
            dev_ctx: DeviceContext::new(31),
            ctrl_ring: Ring::new_producer(64),
            intr_ring: None,
            hid_buf: None,
            port,
            speed,
            intr_ep_id: 0,
            intr_max_pkt: 0,
        }
    }
}

pub struct XhciController {
    pub mmio: MmioRegion,
    pub op_offset: usize,
    rt_offset: usize,
    db_offset: usize,
    pub max_slots: u8,
    pub max_ports: u8,
    pub cmd_ring: Ring,
    pub event_ring: Ring,
    _erst: DmaRegion,
    pub dcbaap: DmaRegion,
    pub slots: Vec<Option<Box<SlotData>>>,
}

impl XhciController {
    pub fn rt_read32(&self, off: usize) -> u32 {
        unsafe { self.mmio.read::<u32>(self.rt_offset + off) }
    }
    pub fn rt_write32(&self, off: usize, val: u32) {
        unsafe { self.mmio.write::<u32>(self.rt_offset + off, val) }
    }
    pub fn rt_write64(&self, off: usize, val: u64) {
        unsafe { self.mmio.write::<u64>(self.rt_offset + off, val) }
    }

    /// Ring a doorbell. `slot` = 0 for the command ring, 1..=max_slots for devices.
    pub fn doorbell(&self, slot: usize, target: u32) {
        unsafe { self.mmio.write::<u32>(self.db_offset + slot * 4, target) }
    }

    /// Read PORTSC for a 1-based port number.
    pub fn portsc_read(&self, port: u8) -> u32 {
        let off = self.op_offset + PORT_BASE + (port as usize - 1) * PORT_STRIDE + PORT_PORTSC;
        unsafe { self.mmio.read::<u32>(off) }
    }

    /// Acknowledge (clear) specific w1c bits in PORTSC.
    pub fn portsc_ack(&self, port: u8, w1c_bits: u32) {
        let current = self.portsc_read(port);
        let off = self.op_offset + PORT_BASE + (port as usize - 1) * PORT_STRIDE + PORT_PORTSC;
        unsafe {
            self.mmio.write::<u32>(
                off,
                (current & !PORTSC_W1C_MASK) | (w1c_bits & PORTSC_W1C_MASK),
            )
        }
    }

    pub fn iman_read(&self) -> u32 {
        self.rt_read32(RT_IR0 + IR_IMAN)
    }
    pub fn iman_write(&self, val: u32) {
        self.rt_write32(RT_IR0 + IR_IMAN, val)
    }
    pub fn erdp_write(&self, val: u64) {
        self.rt_write64(RT_IR0 + IR_ERDP, val)
    }

    pub fn ack_event_ring(&self) {
        let erdp = self.event_ring.dequeue_phys() | ERDP_EHB;
        self.erdp_write(erdp);
    }

    /// Push a TRB onto the command ring and ring doorbell 0.
    pub fn push_cmd(&mut self, trb: Trb) {
        self.cmd_ring.push(trb);
        fence(Ordering::SeqCst);
        self.doorbell(0, 0);
    }

    /// Push transfer TRBs for EP0 and ring the slot doorbell.
    pub fn push_ctrl_trbs(&mut self, slot_id: u8, trbs: &[Trb]) {
        if let Some(Some(slot)) = self.slots.get_mut(slot_id as usize) {
            for &t in trbs {
                slot.ctrl_ring.push(t);
            }
            fence(Ordering::SeqCst);
            self.doorbell(slot_id as usize, 1);
        }
    }

    /// Push a Normal TRB for the HID interrupt-IN endpoint and ring its doorbell.
    pub fn push_intr_trb(&mut self, slot_id: u8, trb: Trb) {
        let ep_id = self
            .slots
            .get(slot_id as usize)
            .and_then(|s| s.as_ref())
            .map(|s| s.intr_ep_id);

        if let (Some(Some(slot)), Some(ep_id)) = (self.slots.get_mut(slot_id as usize), ep_id)
            && let Some(ring) = &mut slot.intr_ring
        {
            ring.push(trb);
            fence(Ordering::SeqCst);
            self.doorbell(slot_id as usize, ep_id as u32);
        }
    }

    /// Store a device context physical address in DCBAAP[slot_id].
    pub fn set_dcbaap_slot(&mut self, slot_id: u8, ctx_phys: u64) {
        let ptr = (self.dcbaap.virt_addr() + (slot_id as usize) * 8) as *mut u64;
        unsafe { core::ptr::write_volatile(ptr, ctx_phys) };
    }

    /// Fully initialize an xHCI controller at the given PCI function.
    /// Returns `None` if the controller cannot be started.
    pub fn init(bus: u8, device: u8, function: u8) -> Option<XhciController> {
        let mut handle = PcieFunction::new(bus, device, function);

        let cmd = handle.read_config_space(0x4)?;
        handle.write_config_space(0x4, cmd | 0x0006);

        let mmio = map_bar(&mut handle, 0)?;

        let cap_len = unsafe { mmio.read::<u8>(CAP_CAPLENGTH) } as usize;
        let hcsparams1 = unsafe { mmio.read::<u32>(CAP_HCSPARAMS1) };
        let dboff = unsafe { mmio.read::<u32>(CAP_DBOFF) } as usize;
        let rtsoff = unsafe { mmio.read::<u32>(CAP_RTSOFF) } as usize;

        let max_slots = (hcsparams1 & HCSPARAMS1_MAX_SLOTS_MASK) as u8;
        let max_ports =
            ((hcsparams1 & HCSPARAMS1_MAX_PORTS_MASK) >> HCSPARAMS1_MAX_PORTS_SHIFT) as u8;

        kprintln!(
            "xhci: max_slots={} max_ports={}",
            max_slots,
            max_ports
        );

        let read_op32 = |off: usize| -> u32 { unsafe { mmio.read::<u32>(cap_len + off) } };
        let write_op32 = |off: usize, val: u32| unsafe { mmio.write::<u32>(cap_len + off, val) };
        let write_op64 = |off: usize, val: u64| unsafe { mmio.write::<u64>(cap_len + off, val) };
        let write_rt32 = |off: usize, val: u32| unsafe { mmio.write::<u32>(rtsoff + off, val) };
        let write_rt64 = |off: usize, val: u64| unsafe { mmio.write::<u64>(rtsoff + off, val) };

        let mut attempts = 0usize;
        while read_op32(OP_USBSTS) & USBSTS_CNR != 0 {
            core::hint::spin_loop();
            attempts += 1;
            if attempts > 1_000_000 {
                kprintln!("xhci: controller never became ready");
                return None;
            }
        }

        write_op32(OP_USBCMD, read_op32(OP_USBCMD) & !USBCMD_RUN);
        let mut attempts = 0usize;
        while read_op32(OP_USBSTS) & USBSTS_HCH == 0 {
            core::hint::spin_loop();
            attempts += 1;
            if attempts > 1_000_000 {
                kprintln!("xhci: controller did not halt");
                return None;
            }
        }

        write_op32(OP_USBCMD, read_op32(OP_USBCMD) | USBCMD_HCRST);
        let mut attempts = 0usize;
        while read_op32(OP_USBCMD) & USBCMD_HCRST != 0 {
            core::hint::spin_loop();
            attempts += 1;
            if attempts > 1_000_000 {
                kprintln!("xhci: reset did not complete");
                return None;
            }
        }
        while read_op32(OP_USBSTS) & USBSTS_CNR != 0 {
            core::hint::spin_loop();
        }

        kprintln!("xhci: reset OK");

        let config = read_op32(OP_CONFIG);
        write_op32(OP_CONFIG, (config & !0xFF) | max_slots as u32);

        let dcbaap = DmaRegion::new_bytes((max_slots as usize + 1) * 8);
        write_op64(OP_DCBAAP, dcbaap.phys_addr() as u64);

        let cmd_ring = Ring::new_producer(256);
        write_op64(OP_CRCR, cmd_ring.phys_base() | 1);

        let event_ring = Ring::new_event(256);
        let erst = alloc_erst(&event_ring);

        write_rt32(RT_IR0 + IR_ERSTSZ, 1);
        write_rt64(RT_IR0 + IR_ERSTBA, erst.phys_addr() as u64);
        write_rt64(RT_IR0 + IR_ERDP, event_ring.phys_base());

        setup_msix(bus, device, function, mmio.virt_addr());

        write_rt32(RT_IR0 + IR_IMAN, IMAN_IE | IMAN_IP);
        write_op32(OP_USBCMD, USBCMD_RUN | USBCMD_INTE | USBCMD_HSEE);

        let mut attempts = 0usize;
        while read_op32(OP_USBSTS) & USBSTS_HCH != 0 {
            core::hint::spin_loop();
            attempts += 1;
            if attempts > 1_000_000 {
                kprintln!("xhci: controller did not start");
                return None;
            }
        }

        kprintln!(
            "xhci: running, max_ports={} max_slots={}",
            max_ports,
            max_slots
        );

        // xHCI spec 4.19.1: if PPC=1, software must set PP=1 before ports will detect devices.
        for port in 1..=max_ports {
            let off = cap_len + PORT_BASE + (port as usize - 1) * PORT_STRIDE + PORT_PORTSC;
            let portsc = unsafe { mmio.read::<u32>(off) };
            kprintln!("xhci: port {} PORTSC=0x{:08x}", port, portsc);
            if portsc & PORTSC_PP == 0 {
                unsafe { mmio.write::<u32>(off, (portsc & !PORTSC_W1C_MASK) | PORTSC_PP) };
                kprintln!("xhci: port {} power enabled", port);
            }
        }

        let mut slots = Vec::with_capacity(max_slots as usize + 1);
        for _ in 0..=max_slots {
            slots.push(None);
        }

        Some(XhciController {
            mmio,
            op_offset: cap_len,
            rt_offset: rtsoff,
            db_offset: dboff,
            max_slots,
            max_ports,
            cmd_ring,
            event_ring,
            _erst: erst,
            dcbaap,
            slots,
        })
    }
}

fn setup_msix(bus: u8, dev: u8, func: u8, bar0_virt: usize) {
    let pcie = PCIE.get().unwrap();

    let cap_ptr_word = match pcie.read_config_space(bus, dev, func, 0x34) {
        Some(v) => v,
        None => return,
    };
    let mut cap_off = (cap_ptr_word & 0xFF) as u16;

    while cap_off != 0 {
        let cap_hdr = match pcie.read_config_space(bus, dev, func, cap_off) {
            Some(v) => v,
            None => break,
        };
        let cap_id = (cap_hdr & 0xFF) as u8;
        let next = ((cap_hdr >> 8) & 0xFF) as u16;

        if cap_id == PCI_CAP_MSIX && enable_msix(bus, dev, func, cap_off, bar0_virt, pcie) {
            return;
        }

        cap_off = next;
    }

    kprintln!("xhci: no MSI-X capability found — interrupts will not work");
}

/// Configure MSI-X: write the first table entry and enable. Returns true on success.
fn enable_msix(
    bus: u8,
    dev: u8,
    func: u8,
    cap_off: u16,
    bar0_virt: usize,
    pcie: &crate::devices::discovery::pcie::Pcie,
) -> bool {
    let mc_word = match pcie.read_config_space(bus, dev, func, cap_off) {
        Some(v) => v,
        None => return false,
    };
    let tbl_bir_off = match pcie.read_config_space(bus, dev, func, cap_off + 4) {
        Some(v) => v,
        None => return false,
    };
    let bir = (tbl_bir_off & 0x7) as usize;
    let table_off = (tbl_bir_off & !0x7) as usize;

    if bir != 0 {
        kprintln!("xhci: MSI-X table in BAR{} — not supported, skipping", bir);
        return false;
    }

    let entry = bar0_virt + table_off;
    unsafe {
        core::ptr::write_volatile(entry as *mut u32, MSI_ADDR_LAPIC0);
        core::ptr::write_volatile((entry + 4) as *mut u32, 0);
        core::ptr::write_volatile((entry + 8) as *mut u32, MSI_DATA_VALUE as u32);
        core::ptr::write_volatile((entry + 12) as *mut u32, 0);
    }

    let new_mc_word = (mc_word | 0x8000_0000) & !0x4000_0000;
    pcie.write_config_space(bus, dev, func, cap_off, new_mc_word);

    kprintln!(
        "xhci: MSI-X enabled (BAR0+0x{:x}) vector=0x{:02x}",
        table_off,
        XHCI_MSI_VECTOR
    );
    true
}

/// Compute the xHCI endpoint ID for a USB endpoint address.
/// EP0 (bidirectional control) always gets ID 1.
pub fn ep_id_from_addr(ep_addr: u8) -> u8 {
    let ep_num = ep_addr & 0x0F;
    if ep_num == 0 {
        1
    } else {
        2 * ep_num + if ep_addr & 0x80 != 0 { 1 } else { 0 }
    }
}

/// Build an 8-byte USB setup packet as a u64 (little-endian).
pub fn make_setup_packet(
    bm_request_type: u8,
    b_request: u8,
    w_value: u16,
    w_index: u16,
    w_length: u16,
) -> u64 {
    (bm_request_type as u64)
        | ((b_request as u64) << 8)
        | ((w_value as u64) << 16)
        | ((w_index as u64) << 32)
        | ((w_length as u64) << 48)
}

/// Spin-yield until the event ring has a Command Completion Event.
/// Returns `(completion_code, slot_id)`. Other events are dispatched while waiting.
pub fn wait_for_cce(ctrl: &IntSpinLock<XhciController>) -> (u8, u8) {
    loop {
        {
            let mut c = ctrl.lock();
            loop {
                match c.event_ring.pop_event() {
                    None => break,
                    Some(trb) => {
                        c.ack_event_ring();
                        let iman = c.rt_read32(RT_IR0 + IR_IMAN);
                        c.rt_write32(RT_IR0 + IR_IMAN, iman | IMAN_IP | IMAN_IE);

                        match trb.trb_type() {
                            trb_type::CMD_COMPLETION => {
                                return (trb.completion_code(), trb.slot_id());
                            }
                            trb_type::PORT_STATUS_CHANGE => {
                                XHCI_IRQ_PENDING.store(true, Ordering::Release);
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        yield_thread();
    }
}

/// Spin-yield until a Transfer Event arrives for `slot_id` and `ep_id`.
/// Returns `(completion_code, bytes_remaining)`.
pub fn wait_for_transfer(ctrl: &IntSpinLock<XhciController>, slot_id: u8, ep_id: u8) -> (u8, u32) {
    loop {
        {
            let mut c = ctrl.lock();
            loop {
                match c.event_ring.pop_event() {
                    None => break,
                    Some(trb) => {
                        c.ack_event_ring();
                        let iman = c.rt_read32(RT_IR0 + IR_IMAN);
                        c.rt_write32(RT_IR0 + IR_IMAN, iman | IMAN_IP | IMAN_IE);

                        match trb.trb_type() {
                            trb_type::TRANSFER_EVENT
                                if trb.slot_id() == slot_id && trb.endpoint_id() == ep_id =>
                            {
                                return (trb.completion_code(), trb.transfer_length_remaining());
                            }
                            trb_type::PORT_STATUS_CHANGE => {
                                XHCI_IRQ_PENDING.store(true, Ordering::Release);
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        yield_thread();
    }
}
