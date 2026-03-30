use crate::devices::device_discovery::{DeviceDiscovery, DeviceNode, DeviceType};
use crate::print::kprintln;
use spin::Once;

pub static PSCI_DEVICE: Once<PSCIDevice> = Once::new();

pub enum PSCIMethod {
    HVC,
    SMC,
}

// https://documentation-service.arm.com/static/6703a8b8d7e4b739d817e10d
// given by the PSCI specification, not needed to be parsed by device discovery
const PSCI_SYSTEM_OFF_FUNC_ID: u32 = 0x8400_0008;

// below are function id's given by device parsing, as they may be different on different platforms. 
// currently not used, all that is needed is shutdown
pub struct PSCIDevice {
    _migrate: u32,
    _cpu_on: u32,
    _cpu_off: u32,
    _cpu_suspend: u32,
    method: PSCIMethod,
}

impl PSCIDevice {
    pub fn new(migrate: u32, cpu_on: u32, cpu_off: u32, cpu_suspend: u32, method: PSCIMethod) -> Self {
        Self {
            _migrate: migrate,
            _cpu_on: cpu_on,
            _cpu_off: cpu_off,
            _cpu_suspend: cpu_suspend,
            method,
        }
    }

    pub fn shutdown(&self) {
        match self.method {
            PSCIMethod::HVC => unsafe { core::arch::asm!("hvc #0", in("x0") PSCI_SYSTEM_OFF_FUNC_ID, options(nostack)) },
            PSCIMethod::SMC => unsafe { core::arch::asm!("smc #0", in("x0") PSCI_SYSTEM_OFF_FUNC_ID, options(nostack)) },
        }
    }
}

pub struct PSCIDiscovery;

impl DeviceDiscovery for PSCIDiscovery {
    fn am_i_this(&self, node: DeviceNode) -> Option<DeviceType> {
        if let DeviceNode::DTB(fdt_node) = node {
            kprintln!("TESTING NODE: {}", fdt_node.name);
        }
        if let DeviceNode::DTB(fdt_node) = node
            && let Some(c) = fdt_node.compatible()
            && c.all().any(|s| matches!(s, "arm,psci" | "arm,psci-0.2" | "arm,psci-1.0"))
        {
            kprintln!("FOUND MATCH");
            let method = fdt_node.property("method")?.as_str()?;
            let psci_method = match method {
                "hvc" => PSCIMethod::HVC,
                "smc" => PSCIMethod::SMC,
                _ => return None,
            };

            // peak rust
            let migrate = u32::from_be_bytes(fdt_node.property("migrate")?.value.try_into().ok()?);
            let cpu_on = u32::from_be_bytes(fdt_node.property("cpu_on")?.value.try_into().ok()?);
            let cpu_off = u32::from_be_bytes(fdt_node.property("cpu_off")?.value.try_into().ok()?);
            let cpu_suspend = u32::from_be_bytes(fdt_node.property("cpu_suspend")?.value.try_into().ok()?);

            PSCI_DEVICE.call_once(|| PSCIDevice::new(migrate, cpu_on, cpu_off, cpu_suspend, psci_method));

            return Some(DeviceType::Special);
        }
        None
    }
}
