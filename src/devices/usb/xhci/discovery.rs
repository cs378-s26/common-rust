use alloc::{vec, vec::Vec};

use crate::{
    devices::{
        discovery::{DeviceDiscovery, DeviceNode, DeviceType},
        usb::xhci::{CONTROLLERS, controller::XhciController},
    },
    print::kprintln,
    sync::MutexLike,
};

const PCI_CLASS_SERIAL_BUS: u8 = 0x0C;
const PCI_SUBCLASS_USB: u8 = 0x03;
const PCI_PROGIF_XHCI: u8 = 0x30;

pub struct XhciDiscovery;

impl DeviceDiscovery for XhciDiscovery {
    fn am_i_this(&self, node: DeviceNode) -> Option<Vec<DeviceType>> {
        let DeviceNode::Pcie(pcie_func) = node else {
            return None;
        };

        let class_reg = pcie_func.read_config_space(0x08)?;
        let prog_if = ((class_reg >> 8) & 0xFF) as u8;
        let subclass = ((class_reg >> 16) & 0xFF) as u8;
        let class_code = ((class_reg >> 24) & 0xFF) as u8;

        if class_code != PCI_CLASS_SERIAL_BUS
            || subclass != PCI_SUBCLASS_USB
            || prog_if != PCI_PROGIF_XHCI
        {
            return None;
        }

        let id_reg = pcie_func.read_config_space(0x00)?;
        let vendor_id = (id_reg & 0xFFFF) as u16;
        let device_id = ((id_reg >> 16) & 0xFFFF) as u16;

        kprintln!(
            "[xhci] found controller {:04x}:{:04x} at {:02x}:{:02x}.{}",
            vendor_id,
            device_id,
            pcie_func.bus,
            pcie_func.device,
            pcie_func.function
        );

        let controller = XhciController::bringup(pcie_func)?;
        CONTROLLERS.lock().push(controller);

        Some(vec![DeviceType::Special])
    }

    fn name(&self) -> &'static str {
        "xhci_discovery"
    }
}
