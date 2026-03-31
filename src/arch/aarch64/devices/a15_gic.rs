use crate::arch::ArchTrait;
use crate::devices::device_discovery::{DeviceDiscovery, DeviceNode, DeviceType};
use crate::virtual_memory::{PagingOptions, VirtualMemoryAllocation};
use core::sync::atomic::{AtomicUsize, Ordering};
use spin::Once;

pub static GICC_BASE_VIRT: AtomicUsize = AtomicUsize::new(0);
pub static GICD_BASE_VIRT: AtomicUsize = AtomicUsize::new(0);
static GICD_INIT: Once<GicA15Driver> = Once::new();

const GICD_CTLR: usize = 0x000;
const GICD_ISENABLER0: usize = 0x100;
const GICD_IPRIORITYR: usize = 0x400;

pub struct GicA15Driver {
    gicd_phys_address: usize,
    gicc_phys_address: usize,
    gicd_virt_mapping: Option<VirtualMemoryAllocation>,
    gicc_virt_mapping: Option<VirtualMemoryAllocation>,
}

impl GicA15Driver {
    fn init(&mut self) -> bool {
        let options =
            PagingOptions::PRESENT | PagingOptions::WRITABLE | PagingOptions::DEVICE_MEMORY;

        let gicd_vm = VirtualMemoryAllocation::new(
            crate::arch::Arch::get_address_space(),
            None,
            crate::arch::Arch::PAGE_SIZE,
            Some(self.gicd_phys_address),
            options,
        );

        let gicc_vm = VirtualMemoryAllocation::new(
            crate::arch::Arch::get_address_space(),
            None,
            crate::arch::Arch::PAGE_SIZE,
            Some(self.gicc_phys_address),
            options,
        );

        if gicd_vm.is_none() || gicc_vm.is_none() {
            panic!("Failed to map GIC memory");
        }

        let gicd_virt = gicd_vm.as_ref().unwrap().base;
        let gicc_virt = gicc_vm.as_ref().unwrap().base;

        self.gicd_virt_mapping = gicd_vm;
        self.gicc_virt_mapping = gicc_vm;

        GICD_BASE_VIRT.store(gicd_virt, Ordering::Release);
        GICC_BASE_VIRT.store(gicc_virt, Ordering::Release);

        unsafe {
            let gicd = gicd_virt as *mut u32;

            // disable gicd
            gicd.add(GICD_CTLR / 4).write_volatile(0);

            // we want intid 30 for the arm's generic timer
            // we need to access the 4 byte register starting at 28 because register 28 includes [28, 29, 30, 31]
            let pri_reg = (gicd_virt + GICD_IPRIORITYR + 28) as *mut u32;

            // read existing config
            let mut word = pri_reg.read_volatile();
            // clear intid 30 priority
            word &= !(0xFF << 16);
            // sets priority to 0xA0
            word |= 0xA0 << 16;
            pri_reg.write_volatile(word);

            // enable interrupt 30 in the Interrupt Set ENABLE Register isENABLEr
            let isenabler0 = gicd.add(GICD_ISENABLER0 / 4);
            isenabler0.write_volatile(1 << 30);

            // reenable gicd
            gicd.add(GICD_CTLR / 4).write_volatile(1);
        }
        true
    }
}

pub struct GicA15Discovery;

impl DeviceDiscovery for GicA15Discovery {
    fn am_i_this(&self, node: DeviceNode<'_, '_>) -> Option<DeviceType> {
        if let DeviceNode::DTB(node) = node
            && let Some(c) = node.compatible()
            && c.all().any(|s| s == "arm,cortex-a15-gic")
            && let Some(mut reg) = node.reg()
        {
            let gicd_phys_address = reg.next().unwrap().starting_address as usize;
            let gicc_phys_address = reg.next().unwrap().starting_address as usize;
            let mut gic = GicA15Driver {
                gicd_phys_address,
                gicc_phys_address,
                gicd_virt_mapping: None,
                gicc_virt_mapping: None,
            };
            gic.init();
            GICD_INIT.call_once(|| gic);
            return Some(DeviceType::Special);
        }
        None
    }

    fn name(&self) -> &'static str {
        "cortex-a15-gic"
    }
}
