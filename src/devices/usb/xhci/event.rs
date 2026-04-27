use alloc::vec::Vec;
use core::sync::atomic::Ordering;

use spin::Lazy;

use super::{
    XHCI, XHCI_IRQ_PENDING,
    device::handle_usb_events,
    regs::{IMAN_IE, IMAN_IP, PORTSC_CCS},
    trb::Trb,
};
use crate::{
    arch::apic,
    print::kprintln,
    sync::{MutexLike, Semaphore},
};

extern crate alloc;

static USB_SEM: Lazy<Semaphore> = Lazy::new(|| Semaphore::new(0));

/// Called by the architecture-level interrupt dispatcher for vector 0x30.
pub fn handle_interrupt() -> Option<()> {
    XHCI_IRQ_PENDING.store(true, Ordering::Release);
    apic::eoi();
    USB_SEM.up();
    Some(())
}

pub fn usb_event_thread() {
    kprintln!("xhci: USB event thread started");
    initial_port_scan();

    loop {
        USB_SEM.down();

        if !XHCI_IRQ_PENDING.swap(false, Ordering::AcqRel) {
            continue;
        }

        let events = drain_event_ring();
        handle_usb_events(events);
    }
}

fn drain_event_ring() -> Vec<Trb> {
    let mut events = Vec::new();

    if let Some(ctrl_lock) = XHCI.get() {
        let mut ctrl = ctrl_lock.lock();
        while let Some(trb) = ctrl.event_ring.pop_event() {
            events.push(trb);
        }
        ctrl.ack_event_ring();
        let iman = ctrl.iman_read();
        ctrl.iman_write(iman | IMAN_IP | IMAN_IE);
    }

    events
}

fn initial_port_scan() {
    let max_ports = XHCI.get().map(|c| c.lock().max_ports).unwrap_or(0);

    for port in 1..=max_ports {
        let portsc = XHCI.get().map(|c| c.lock().portsc_read(port)).unwrap_or(0);
        if portsc & PORTSC_CCS != 0 {
            kprintln!("xhci: device already connected on port {}", port);
            super::device::start_port_reset(port);
        }
    }
}
