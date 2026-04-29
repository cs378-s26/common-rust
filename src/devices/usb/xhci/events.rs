//! Event ring drain — runs from the MSI/MSI-X interrupt handler. The handler
//! stays tiny so all real work happens on whichever thread the dispatched
//! `Promise` wakes.

use alloc::{collections::BTreeMap, sync::Arc};

use crate::{
    devices::usb::xhci::{
        registers::{
            ERDP_EHB, IMAN_IE, IMAN_IP, IR_ERDP_HI, IR_ERDP_LO, IR_IMAN, RT_IR0_BASE,
            TRB_TYPE_COMMAND_COMPLETION_EVENT, TRB_TYPE_PORT_STATUS_CHANGE_EVENT,
            TRB_TYPE_TRANSFER_EVENT, trb_get_type,
        },
        ring::{EventRingState, Trb},
    },
    memory::dma::MmioRegion,
    print::kprintln,
    sync::{IntMutex, MutexLike, Promise},
};

pub type PendingMap = BTreeMap<u64, Arc<Promise<Trb>>>;

pub struct EventRingHandler {
    pub mmio: Arc<MmioRegion>,
    pub runtime_offset: usize,
    pub state: IntMutex<EventRingState>,
    pub pending: IntMutex<PendingMap>,
}

impl EventRingHandler {
    pub fn handle(&self) {
        loop {
            let trb_opt = {
                let mut state = self.state.lock();
                let trb = state.peek();
                if trb.is_some() {
                    state.advance();
                }
                trb
            };
            let Some(trb) = trb_opt else { break };
            self.dispatch(trb);
        }

        let ir0 = self.runtime_offset + RT_IR0_BASE;

        // Writing the new ERDP with bit 3 (EHB, RW1C) set re-arms the
        // interrupter. Without this the HC won't fire again.
        let new_erdp = {
            let state = self.state.lock();
            state.current_phys() | ERDP_EHB
        };
        // SAFETY: ir0 + IR_ERDP_{LO,HI} are inside the BAR; aligned u32 stores.
        unsafe {
            self.mmio
                .write::<u32>(ir0 + IR_ERDP_LO, (new_erdp & 0xFFFF_FFFF) as u32);
            self.mmio
                .write::<u32>(ir0 + IR_ERDP_HI, (new_erdp >> 32) as u32);
            // IMAN.IP is RW1C; writing 1 clears it. IE stays asserted.
            self.mmio.write::<u32>(ir0 + IR_IMAN, IMAN_IE | IMAN_IP);
        }
    }

    fn dispatch(&self, trb: Trb) {
        let trb_type = trb_get_type(trb.data[3]);
        match trb_type {
            t if t == TRB_TYPE_COMMAND_COMPLETION_EVENT || t == TRB_TYPE_TRANSFER_EVENT => {
                let trb_ptr = (trb.data[0] as u64) | ((trb.data[1] as u64) << 32);
                let key = trb_ptr & !0xF;
                let promise = self.pending.lock().remove(&key);
                if let Some(p) = promise {
                    p.set(trb);
                } else {
                    kprintln!("[xhci] event for unknown TRB ptr {:#x}", key);
                }
            }
            // Stage 1 polls PORTSC after triggering reset, so port-status
            // events have no waiter to signal — silently drained here.
            t if t == TRB_TYPE_PORT_STATUS_CHANGE_EVENT => {}
            other => {
                kprintln!("[xhci] unhandled event type {}", other);
            }
        }
    }
}
