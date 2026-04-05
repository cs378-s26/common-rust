use crate::devices::discovery::{
    AcpiDeviceNode::MadtEntry, DeviceNode::Acpi, DeviceType, SYSTEM_DRIVERS,
};
use crate::physical_memory::HHDM_OFFSET;
use crate::print::kprintln;
use crate::sync::MutexLike;
use acpi::aml::Interpreter;
use acpi::platform::AcpiPlatform;
use acpi::{AcpiTables, Handler as AcpiHandler, PhysicalMapping, sdt::madt::Madt};
use alloc::vec::Vec;
use core::ptr::NonNull;
use limine::request::RsdpRequest;
// use virtio_drivers::read_config;

#[used]
#[unsafe(link_section = ".limine_requests")]
static RSDP_REQUEST: RsdpRequest = RsdpRequest::new();

#[derive(Clone)]
struct KernelAcpiHandler;

impl AcpiHandler for KernelAcpiHandler {
    unsafe fn map_physical_region<T>(
        &self,
        physical_address: usize,
        size: usize,
    ) -> PhysicalMapping<Self, T> {
        let virt = physical_address + HHDM_OFFSET.get().unwrap();
        PhysicalMapping {
            physical_start: physical_address,
            virtual_start: NonNull::new(virt as *mut T).unwrap(),
            region_length: size,
            mapped_length: size,
            handler: self.clone(),
        }
    }

    fn unmap_physical_region<T>(_region: &PhysicalMapping<Self, T>) {
        // HHDM mappings are permanent — nothing to do
    }

    fn read_u8(&self, address: usize) -> u8 {
        unsafe { core::ptr::read_volatile((address + HHDM_OFFSET.get().unwrap()) as *const u8) }
    }
    fn read_u16(&self, address: usize) -> u16 {
        unsafe { core::ptr::read_volatile((address + HHDM_OFFSET.get().unwrap()) as *const u16) }
    }
    fn read_u32(&self, address: usize) -> u32 {
        unsafe { core::ptr::read_volatile((address + HHDM_OFFSET.get().unwrap()) as *const u32) }
    }
    fn read_u64(&self, address: usize) -> u64 {
        unsafe { core::ptr::read_volatile((address + HHDM_OFFSET.get().unwrap()) as *const u64) }
    }

    fn write_u8(&self, address: usize, value: u8) {
        unsafe {
            core::ptr::write_volatile((address + HHDM_OFFSET.get().unwrap()) as *mut u8, value)
        }
    }
    fn write_u16(&self, address: usize, value: u16) {
        unsafe {
            core::ptr::write_volatile((address + HHDM_OFFSET.get().unwrap()) as *mut u16, value)
        }
    }
    fn write_u32(&self, address: usize, value: u32) {
        unsafe {
            core::ptr::write_volatile((address + HHDM_OFFSET.get().unwrap()) as *mut u32, value)
        }
    }
    fn write_u64(&self, address: usize, value: u64) {
        unsafe {
            core::ptr::write_volatile((address + HHDM_OFFSET.get().unwrap()) as *mut u64, value)
        }
    }

    fn read_io_u8(&self, port: u16) -> u8 {
        unsafe { x86::io::inb(port) }
    }
    fn read_io_u16(&self, port: u16) -> u16 {
        unsafe { x86::io::inw(port) }
    }
    fn read_io_u32(&self, port: u16) -> u32 {
        unsafe { x86::io::inl(port) }
    }

    fn write_io_u8(&self, port: u16, value: u8) {
        unsafe { x86::io::outb(port, value) }
    }
    fn write_io_u16(&self, port: u16, value: u16) {
        unsafe { x86::io::outw(port, value) }
    }
    fn write_io_u32(&self, port: u16, value: u32) {
        unsafe { x86::io::outl(port, value) }
    }

    fn read_pci_u8(&self, _address: acpi::PciAddress, _offset: u16) -> u8 {
        unimplemented!("PCI not yet supported")
    }
    fn read_pci_u16(&self, _address: acpi::PciAddress, _offset: u16) -> u16 {
        unimplemented!("PCI not yet supported")
    }
    fn read_pci_u32(&self, _address: acpi::PciAddress, _offset: u16) -> u32 {
        unimplemented!("PCI not yet supported")
    }

    fn write_pci_u8(&self, _address: acpi::PciAddress, _offset: u16, _value: u8) {
        unimplemented!("PCI not yet supported")
    }
    fn write_pci_u16(&self, _address: acpi::PciAddress, _offset: u16, _value: u16) {
        unimplemented!("PCI not yet supported")
    }
    fn write_pci_u32(&self, _address: acpi::PciAddress, _offset: u16, _value: u32) {
        unimplemented!("PCI not yet supported")
    }

    fn nanos_since_boot(&self) -> u64 {
        0
    }

    fn stall(&self, _microseconds: u64) {}

    fn sleep(&self, _milliseconds: u64) {}

    fn create_mutex(&self) -> acpi::Handle {
        acpi::Handle(0)
    }

    fn acquire(&self, _mutex: acpi::Handle, _timeout: u16) -> Result<(), acpi::aml::AmlError> {
        Ok(())
    }

    fn release(&self, _mutex: acpi::Handle) {}
}

// fn match_device(node: AcpiDeviceNode) {
//     let drivers = SYSTEM_DRIVERS.lock();
//     for driver in drivers.iter() {
//         if let Some(device) = driver.am_i_this(DeviceNode::Acpi(node)) {
//             match device {
//                 DeviceType::Block(d) => BLOCK_DEVICES.lock().push(d),
//                 DeviceType::Char(d) => CHAR_DEVICES.lock().push(d),
//                 DeviceType::Network(d) => NETWORK_DEVICES.lock().push(d),
//                 DeviceType::Special => {}
//             }
//             return;
//         }
//     }
// }

pub fn parse_acpi() -> Option<Vec<DeviceType>> {
    let rsdp = RSDP_REQUEST.get_response()?.address();
    let tables = unsafe {
        AcpiTables::from_rsdp(KernelAcpiHandler, rsdp - HHDM_OFFSET.get().unwrap()).ok()?
    };
    let mut matched_devices = Vec::new();

    // Walk MADT for I/O APIC entries
    if let Some(madt) = tables.find_table::<Madt>() {
        for driver in SYSTEM_DRIVERS.lock().iter() {
            for entry in madt.get().entries() {
                let matched_device = driver.am_i_this(Acpi(MadtEntry(entry)));
                if let Some(device) = matched_device {
                    matched_devices.push(device);
                }
            }
        }
    } else {
        kprintln!("[devices] no MADT found");
    }

    // Walk the DSDT
    let platform = AcpiPlatform::new(tables, KernelAcpiHandler).ok()?;
    let interpreter = Interpreter::new_from_platform(&platform).ok()?;
    interpreter.initialize_namespace();
    interpreter
        .namespace
        .lock()
        .traverse(|name, _| {
            kprintln!("Device {}", name.as_string());
            Ok(true)
        })
        .ok()?;

    Some(matched_devices)
}
