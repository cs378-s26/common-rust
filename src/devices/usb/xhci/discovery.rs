use alloc::{vec, vec::Vec};

use super::{XHCI, XhciController, event::usb_event_thread};
use crate::{
    devices::discovery::{DeviceDiscovery, DeviceNode, DeviceType, pcie::PCIE},
    print::kprintln,
    thread::spawn_thread,
};

pub struct XhciDiscovery;

impl DeviceDiscovery for XhciDiscovery {
    fn am_i_this(&self, node: DeviceNode) -> Option<Vec<DeviceType>> {
        let DeviceNode::Pcie(f) = node else {
            return None;
        };

        // Bits [31:8] of config offset 0x08: base class 0x0C (Serial Bus),
        // subclass 0x03 (USB), prog IF 0x30 (xHCI).
        let class_reg = PCIE
            .get()?
            .read_config_space(f.bus, f.device, f.function, 0x08)?;
        if (class_reg >> 8) & 0x00FF_FFFF != 0x0C_03_30 {
            return None;
        }

        kprintln!(
            "xhci: found controller at {:02x}:{:02x}.{}",
            f.bus,
            f.device,
            f.function
        );

        let ctrl = XhciController::init(f.bus, f.device, f.function)?;
        XHCI.call_once(|| crate::sync::IntSpinLock::new(ctrl));
        spawn_thread(usb_event_thread);

        Some(vec![DeviceType::Special])
    }

    fn name(&self) -> &'static str {
        "xhci"
    }
}
