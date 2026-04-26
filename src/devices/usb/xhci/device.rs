extern crate alloc;

use alloc::{boxed::Box, vec::Vec};

use super::{
    SlotData, XHCI,
    context::{InputContext, ep_type},
    ep_id_from_addr, make_setup_packet,
    regs::*,
    ring::Ring,
    trb::{Trb, cc, trb_type},
    wait_for_cce, wait_for_transfer,
};
use crate::{memory::dma::DmaRegion, print::kprintln, sync::MutexLike};

const DESC_TYPE_DEVICE: u16 = 0x01;
const DESC_TYPE_CONFIG: u16 = 0x02;
const DESC_TYPE_INTERFACE: u8 = 0x04;
const DESC_TYPE_ENDPOINT: u8 = 0x05;

const REQ_GET_DESCRIPTOR: u8 = 0x06;
const REQ_SET_CONFIGURATION: u8 = 0x09;
const REQ_HID_SET_PROTOCOL: u8 = 0x0B;
const REQ_HID_SET_IDLE: u8 = 0x0A;

const BM_DEVICE_IN: u8 = 0x80;
const BM_HID_CLASS_OUT: u8 = 0x21;

const HID_CLASS: u8 = 0x03;
const HID_SUBCLASS_BOOT: u8 = 0x01;
const HID_PROTO_KBD: u8 = 0x01;
const HID_PROTO_MOUSE: u8 = 0x02;

// HID boot-protocol keycode to ASCII (unshifted)
static HID_KEYS_NORMAL: [u8; 0x39] = [
    0, 0, 0, 0, b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i', b'j', b'k', b'l', b'm', b'n',
    b'o', b'p', b'q', b'r', b's', b't', b'u', b'v', b'w', b'x', b'y', b'z', b'1', b'2', b'3', b'4',
    b'5', b'6', b'7', b'8', b'9', b'0', b'\n', 0x1B, 0x08, b'\t', b' ', b'-', b'=', b'[', b']',
    b'\\', b'#', b';', b'\'', b'`', b',', b'.', b'/',
];

// HID boot-protocol keycode  to ASCII (shifted)
static HID_KEYS_SHIFTED: [u8; 0x39] = [
    0, 0, 0, 0, b'A', b'B', b'C', b'D', b'E', b'F', b'G', b'H', b'I', b'J', b'K', b'L', b'M', b'N',
    b'O', b'P', b'Q', b'R', b'S', b'T', b'U', b'V', b'W', b'X', b'Y', b'Z', b'!', b'@', b'#', b'$',
    b'%', b'^', b'&', b'*', b'(', b')', b'\n', 0x1B, 0x08, b'\t', b' ', b'_', b'+', b'{', b'}',
    b'|', b'~', b':', b'"', b'~', b'<', b'>', b'?',
];

fn keycode_to_ascii(keycode: u8, shifted: bool) -> u8 {
    let table = if shifted {
        &HID_KEYS_SHIFTED
    } else {
        &HID_KEYS_NORMAL
    };
    *table.get(keycode as usize).unwrap_or(&0)
}

fn ep0_max_pkt(speed: u32) -> u16 {
    match speed {
        SPEED_LOW => 8,
        SPEED_SUPER | SPEED_SUPER_PLUS => 512,
        _ => 64,
    }
}

fn get_desc_trbs(setup_pkt: u64, len: u16, buf_phys: u64) -> [Trb; 3] {
    [
        Trb::setup_stage(setup_pkt, false),
        Trb::data_stage_in(buf_phys, len, false),
        Trb::status_stage_out(false),
    ]
}

fn ctrl_out_trbs(setup_pkt: u64) -> [Trb; 2] {
    [
        Trb::setup_stage(setup_pkt, false),
        Trb::status_stage_in(false),
    ]
}

pub fn handle_usb_events(events: Vec<Trb>) {
    for trb in events {
        match trb.trb_type() {
            trb_type::PORT_STATUS_CHANGE => handle_psc(trb.port_id()),
            trb_type::TRANSFER_EVENT => handle_hid_transfer(trb),
            _ => {}
        }
    }
}

fn handle_psc(port: u8) {
    let portsc = match XHCI.get() {
        Some(c) => c.lock().portsc_read(port),
        None => return,
    };
    kprintln!("xhci: port {} PSC PORTSC=0x{:08x}", port, portsc);

    if let Some(c) = XHCI.get() {
        c.lock().portsc_ack(port, PORTSC_W1C_MASK);
    }

    if portsc & PORTSC_PRC != 0 && portsc & PORTSC_PED != 0 {
        let speed = (portsc & PORTSC_SPEED_MASK) >> PORTSC_SPEED_SHIFT;
        enumerate_device(port, speed);
    } else if portsc & PORTSC_CSC != 0 {
        if portsc & PORTSC_CCS != 0 {
            start_port_reset(port);
        } else {
            handle_disconnect(port);
        }
    }
}

pub fn start_port_reset(port: u8) {
    if let Some(c) = XHCI.get() {
        let ctrl = c.lock();
        let portsc = ctrl.portsc_read(port);
        let val = (portsc & !PORTSC_W1C_MASK) | PORTSC_PR;
        let off = ctrl.op_offset + PORT_BASE + (port as usize - 1) * PORT_STRIDE + PORT_PORTSC;
        unsafe { ctrl.mmio.write::<u32>(off, val) };
        kprintln!("xhci: port {} reset initiated", port);
    }
}

fn handle_disconnect(port: u8) {
    if let Some(c) = XHCI.get() {
        let mut ctrl = c.lock();
        for slot in ctrl.slots.iter_mut() {
            if let Some(s) = slot
                && s.port == port
            {
                kprintln!("xhci: port {} disconnect", port);
                *slot = None;
                return;
            }
        }
    }
}

fn enumerate_device(port: u8, speed: u32) {
    kprintln!("xhci: enumerating port {} speed={}", port, speed);

    {
        let mut ctrl = XHCI.get().unwrap().lock();
        let c = ctrl.cmd_ring.cycle;
        ctrl.push_cmd(Trb::enable_slot(c));
    }
    let (code, slot_id) = wait_for_cce(XHCI.get().unwrap());
    if code != cc::SUCCESS || slot_id == 0 {
        kprintln!("xhci: enable_slot failed cc={}", code);
        return;
    }
    kprintln!("xhci: slot {} enabled for port {}", slot_id, port);

    let slot_data = Box::new(SlotData::new(port, speed));
    let dev_ctx_phys = slot_data.dev_ctx.phys();
    let ctrl_ring_phys = slot_data.ctrl_ring.phys_base();
    let ctrl_ring_dcs = slot_data.ctrl_ring.cycle;

    {
        let mut ctrl = XHCI.get().unwrap().lock();
        ctrl.set_dcbaap_slot(slot_id, dev_ctx_phys);
        while ctrl.slots.len() <= slot_id as usize {
            ctrl.slots.push(None);
        }
        ctrl.slots[slot_id as usize] = Some(slot_data);
    }

    let mut ic = InputContext::new(1);
    ic.icc_mut().add_flags = 0b11; // A0 (slot) | A1 (EP0)
    ic.slot_mut().set_speed(speed);
    ic.slot_mut().set_root_hub_port(port);
    ic.slot_mut().set_context_entries(1);
    {
        let ep0 = ic.ep_mut(0);
        ep0.set_ep_type(ep_type::CONTROL);
        ep0.set_max_packet_size(ep0_max_pkt(speed));
        ep0.set_cerr(3);
        ep0.set_tr_dequeue_ptr(ctrl_ring_phys, ctrl_ring_dcs);
        ep0.set_avg_trb_length(8);
    }
    {
        let mut ctrl = XHCI.get().unwrap().lock();
        let c = ctrl.cmd_ring.cycle;
        ctrl.push_cmd(Trb::address_device(ic.phys(), slot_id, c));
    }
    let (code, _) = wait_for_cce(XHCI.get().unwrap());
    if code != cc::SUCCESS {
        kprintln!("xhci: address_device failed cc={}", code);
        return;
    }
    kprintln!("xhci: slot {} addressed", slot_id);

    let dev_buf = DmaRegion::new_bytes(18);
    {
        let pkt = make_setup_packet(
            BM_DEVICE_IN,
            REQ_GET_DESCRIPTOR,
            DESC_TYPE_DEVICE << 8,
            0,
            18,
        );
        let trbs = get_desc_trbs(pkt, 18, dev_buf.phys_addr() as u64);
        XHCI.get().unwrap().lock().push_ctrl_trbs(slot_id, &trbs);
    }
    let (code, _) = wait_for_transfer(XHCI.get().unwrap(), slot_id, 1);
    if code != cc::SUCCESS && code != cc::SHORT_PACKET {
        kprintln!("xhci: GET_DESCRIPTOR(device) failed cc={}", code);
        return;
    }
    let d = dev_buf.as_slice();
    let vid = u16::from_le_bytes([d[8], d[9]]);
    let pid = u16::from_le_bytes([d[10], d[11]]);
    kprintln!("xhci: USB device vid={:04x} pid={:04x}", vid, pid);

    let cfg_buf = DmaRegion::new_bytes(255);
    {
        let pkt = make_setup_packet(
            BM_DEVICE_IN,
            REQ_GET_DESCRIPTOR,
            DESC_TYPE_CONFIG << 8,
            0,
            255,
        );
        let trbs = get_desc_trbs(pkt, 255, cfg_buf.phys_addr() as u64);
        XHCI.get().unwrap().lock().push_ctrl_trbs(slot_id, &trbs);
    }
    let (code, _) = wait_for_transfer(XHCI.get().unwrap(), slot_id, 1);
    if code != cc::SUCCESS && code != cc::SHORT_PACKET {
        kprintln!("xhci: GET_DESCRIPTOR(config) failed cc={}", code);
        return;
    }

    let cfg = cfg_buf.as_slice();
    let total_len = u16::from_le_bytes([cfg[2], cfg[3]]) as usize;
    let config_value = cfg[5];

    let Some((iface_num, ep_addr, ep_mps, ep_interval, hid_proto)) =
        parse_hid_endpoint(cfg, total_len)
    else {
        kprintln!("xhci: no HID boot endpoint found");
        return;
    };
    kprintln!(
        "xhci: HID proto={} iface={} ep=0x{:02x} mps={}",
        hid_proto,
        iface_num,
        ep_addr,
        ep_mps
    );

    {
        let pkt = make_setup_packet(0x00, REQ_SET_CONFIGURATION, config_value as u16, 0, 0);
        let trbs = ctrl_out_trbs(pkt);
        XHCI.get().unwrap().lock().push_ctrl_trbs(slot_id, &trbs);
    }
    let (code, _) = wait_for_transfer(XHCI.get().unwrap(), slot_id, 1);
    if code != cc::SUCCESS {
        kprintln!("xhci: SET_CONFIGURATION failed cc={}", code);
        return;
    }

    {
        let pkt = make_setup_packet(
            BM_HID_CLASS_OUT,
            REQ_HID_SET_PROTOCOL,
            0,
            iface_num as u16,
            0,
        );
        let trbs = ctrl_out_trbs(pkt);
        XHCI.get().unwrap().lock().push_ctrl_trbs(slot_id, &trbs);
    }
    let _ = wait_for_transfer(XHCI.get().unwrap(), slot_id, 1);

    {
        let pkt = make_setup_packet(BM_HID_CLASS_OUT, REQ_HID_SET_IDLE, 0, iface_num as u16, 0);
        let trbs = ctrl_out_trbs(pkt);
        XHCI.get().unwrap().lock().push_ctrl_trbs(slot_id, &trbs);
    }
    let _ = wait_for_transfer(XHCI.get().unwrap(), slot_id, 1);

    let intr_ep_id = ep_id_from_addr(ep_addr);
    let intr_ring = Ring::new_producer(64);
    let intr_ring_phys = intr_ring.phys_base();
    let intr_ring_dcs = intr_ring.cycle;
    let hid_buf = DmaRegion::new_bytes(ep_mps as usize);
    let hid_buf_phys = hid_buf.phys_addr() as u64;

    {
        let mut ctrl = XHCI.get().unwrap().lock();
        if let Some(Some(slot)) = ctrl.slots.get_mut(slot_id as usize) {
            slot.intr_ring = Some(intr_ring);
            slot.hid_buf = Some(hid_buf);
            slot.intr_ep_id = intr_ep_id;
            slot.intr_max_pkt = ep_mps;
        }
    }

    let mut cic = InputContext::new(intr_ep_id as usize);
    cic.icc_mut().add_flags = (1 << 0) | (1 << intr_ep_id);
    cic.slot_mut().set_speed(speed);
    cic.slot_mut().set_root_hub_port(port);
    cic.slot_mut().set_context_entries(intr_ep_id);
    {
        let ep = cic.ep_mut(intr_ep_id as usize - 1);
        ep.set_ep_type(ep_type::INTERRUPT_IN);
        ep.set_max_packet_size(ep_mps);
        ep.set_cerr(3);
        ep.set_hid();
        ep.set_interval(convert_interval(speed, ep_interval));
        ep.set_tr_dequeue_ptr(intr_ring_phys, intr_ring_dcs);
        ep.set_avg_trb_length(ep_mps);
    }
    {
        let mut ctrl = XHCI.get().unwrap().lock();
        let c = ctrl.cmd_ring.cycle;
        ctrl.push_cmd(Trb::configure_endpoint(cic.phys(), slot_id, c));
    }
    let (code, _) = wait_for_cce(XHCI.get().unwrap());
    if code != cc::SUCCESS {
        kprintln!("xhci: configure_endpoint failed cc={}", code);
        return;
    }

    kprintln!(
        "xhci: {} ready (slot={} ep_id={})",
        if hid_proto == HID_PROTO_KBD {
            "keyboard"
        } else {
            "mouse"
        },
        slot_id,
        intr_ep_id
    );

    post_hid_trb(slot_id, hid_buf_phys, ep_mps as u32);
}

fn handle_hid_transfer(trb: Trb) {
    let slot_id = trb.slot_id();
    let ep_id = trb.endpoint_id();
    let code = trb.completion_code();

    let (buf_phys, mps, ok) = {
        let ctrl = XHCI.get().unwrap().lock();
        let slot = match ctrl.slots.get(slot_id as usize).and_then(|s| s.as_ref()) {
            Some(s) => s,
            None => return,
        };
        if slot.intr_ep_id != ep_id {
            return;
        }
        let phys = slot
            .hid_buf
            .as_ref()
            .map(|b| b.phys_addr() as u64)
            .unwrap_or(0);
        (
            phys,
            slot.intr_max_pkt,
            code == cc::SUCCESS || code == cc::SHORT_PACKET,
        )
    };

    if buf_phys == 0 {
        return;
    }

    if ok {
        let report: Vec<u8> = {
            let ctrl = XHCI.get().unwrap().lock();
            match ctrl.slots.get(slot_id as usize).and_then(|s| s.as_ref()) {
                Some(s) => s
                    .hid_buf
                    .as_ref()
                    .map(|b| b.as_slice()[..(mps as usize).min(8)].to_vec())
                    .unwrap_or_default(),
                None => return,
            }
        };
        decode_hid_report(slot_id, &report);
    } else {
        kprintln!("xhci: HID transfer error cc={} slot={}", code, slot_id);
    }

    post_hid_trb(slot_id, buf_phys, mps as u32);
}

fn post_hid_trb(slot_id: u8, buf_phys: u64, len: u32) {
    let trb = Trb::normal(buf_phys, len, false);
    XHCI.get().unwrap().lock().push_intr_trb(slot_id, trb);
}

fn decode_hid_report(slot_id: u8, report: &[u8]) {
    if report.is_empty() {
        return;
    }
    if report.len() >= 8 {
        let mods = report[0];
        let shifted = mods & 0x22 != 0; // left or right Shift
        for &k in report[2..8].iter().filter(|&&k| k != 0) {
            let ch = keycode_to_ascii(k, shifted);
            if ch.is_ascii_graphic() || ch == b' ' {
                kprintln!("xhci: Keyboard slot={} '{}'", slot_id, ch as char);
            } else if ch != 0 {
                kprintln!("xhci: Keyboard slot={} ctrl key=0x{:02x}", slot_id, k);
            } else {
                kprintln!(
                    "xhci: Keyboard slot={} mod=0x{:02x} key=0x{:02x}",
                    slot_id,
                    mods,
                    k
                );
            }
        }
    } else if report.len() >= 3 {
        let btn = report[0];
        let dx = report[1] as i8;
        let dy = report[2] as i8;
        if btn != 0 || dx != 0 || dy != 0 {
            kprintln!(
                "xhci: Mouse slot={} btn={} dx={} dy={}",
                slot_id,
                btn,
                dx,
                dy
            );
        }
    }
}

fn parse_hid_endpoint(data: &[u8], total: usize) -> Option<(u8, u8, u16, u8, u8)> {
    let end = total.min(data.len());
    let mut i = 0;
    let mut iface: u8 = 0;
    let mut hid_proto: u8 = 0;
    let mut in_hid_boot = false;

    while i < end {
        let len = data[i] as usize;
        if len < 2 || i + len > end {
            break;
        }
        match data[i + 1] {
            t if t == DESC_TYPE_INTERFACE && len >= 9 => {
                iface = data[i + 2];
                let class = data[i + 5];
                let sub = data[i + 6];
                hid_proto = data[i + 7];
                in_hid_boot = class == HID_CLASS
                    && sub == HID_SUBCLASS_BOOT
                    && (hid_proto == HID_PROTO_KBD || hid_proto == HID_PROTO_MOUSE);
            }
            t if t == DESC_TYPE_ENDPOINT && in_hid_boot && len >= 7 => {
                let addr = data[i + 2];
                let attr = data[i + 3];
                let mps = u16::from_le_bytes([data[i + 4], data[i + 5]]);
                let interval = data[i + 6];
                if addr & 0x80 != 0 && attr & 0x03 == 0x03 {
                    return Some((iface, addr, mps, interval, hid_proto));
                }
            }
            _ => {}
        }
        i += len;
    }
    None
}

fn convert_interval(speed: u32, binterval: u8) -> u8 {
    match speed {
        SPEED_HIGH | SPEED_SUPER | SPEED_SUPER_PLUS => binterval.saturating_sub(1).min(15),
        _ => {
            let units = (binterval as u32).max(1) * 8;
            (31u32.saturating_sub(units.leading_zeros())).min(15) as u8
        }
    }
}
