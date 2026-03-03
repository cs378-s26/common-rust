use crate::device::pci::Pci;
use crate::print::kprintln;
//PRESENT, GLOBAL are bitflags.

use crate::virtual_memory::{VirtualMemoryAllocation, PagingOptions};
use crate::physical_memory::HHDM_OFFSET;
use crate::arch::{Arch, ArchTrait};
use acpi::{
    AcpiTables, Handler, PhysicalMapping,
    aml::AmlError,
    sdt::{
        // bgrt::Bgrt,
        // facs::Facs,
        // fadt::Fadt,
        hpet::{HpetInfo, HpetTable},
        madt::{Madt, MadtEntry},
        mcfg::Mcfg,
        // slit::Slit,
        // spcr::Spcr,
        // srat::Srat,
    },
};
use core::{
    hint::spin_loop,
    pin::Pin,
    ptr::{NonNull, read_volatile, write_volatile},
};
use spin::Once;
use x86::{
    io::{inb, inl, inw, outb, outl, outw},
    time::rdtsc,
};

use alloc::vec::Vec;
pub struct KernelAcpiHandler {
}

impl Handler for KernelAcpiHandler {
    unsafe fn map_physical_region<T>(
        &self,
        physical_address: usize,
        size: usize,
    ) -> PhysicalMapping<Self, T> {
        // Add if < HHDM offset (haven't already added).
        let vaddr = if physical_address < *HHDM_OFFSET.get().unwrap() {
            physical_address + HHDM_OFFSET.get().unwrap()
        } else {
            physical_address
        };
        PhysicalMapping {
            physical_start: physical_address,
            virtual_start: NonNull::new(vaddr as *mut T)
                .expect("Failed to get virtual address."),
            region_length: size,
            mapped_length: size,
            handler: KernelAcpiHandler {}
        }
    }

    //Drop should handle things for us question mark?
    fn unmap_physical_region<T>(region: &PhysicalMapping<Self, T>) {} // Nothing to unmap.

    fn read_u8(&self, address: usize) -> u8 {
        unsafe { read_volatile((address + HHDM_OFFSET.get().unwrap()) as *const u8) }
    }
    fn read_u16(&self, address: usize) -> u16 {
        unsafe { read_volatile((address + HHDM_OFFSET.get().unwrap()) as *const u16) }
    }
    fn read_u32(&self, address: usize) -> u32 {
        unsafe { read_volatile((address + HHDM_OFFSET.get().unwrap()) as *const u32) }
    }
    fn read_u64(&self, address: usize) -> u64 {
        unsafe { read_volatile((address + HHDM_OFFSET.get().unwrap()) as *const u64) }
    }

    fn write_u8(&self, address: usize, value: u8) {
        unsafe { write_volatile((address + HHDM_OFFSET.get().unwrap()) as *mut u8, value) }
    }
    fn write_u16(&self, address: usize, value: u16) {
        unsafe { write_volatile((address + HHDM_OFFSET.get().unwrap()) as *mut u16, value) }
    }
    fn write_u32(&self, address: usize, value: u32) {
        unsafe { write_volatile((address + HHDM_OFFSET.get().unwrap()) as *mut u32, value) }
    }
    fn write_u64(&self, address: usize, value: u64) {
        unsafe { write_volatile((address + HHDM_OFFSET.get().unwrap()) as *mut u64, value) }
    }

    // TODO: all of this.
    fn read_io_u8(&self, port: u16) -> u8 {
        unsafe { inb(port) }
    }
    fn read_io_u16(&self, port: u16) -> u16 {
        unsafe { inw(port) }
    }
    fn read_io_u32(&self, port: u16) -> u32 {
        unsafe { inl(port) }
    }

    fn write_io_u8(&self, port: u16, value: u8) {
        unsafe { outb(port, value) }
    }
    fn write_io_u16(&self, port: u16, value: u16) {
        unsafe { outw(port, value) }
    }
    fn write_io_u32(&self, port: u16, value: u32) {
        unsafe { outl(port, value) }
    }

    fn read_pci_u8(&self, address: acpi::PciAddress, offset: u16) -> u8 {
        Pci::read_u8(
            address.bus(),
            address.device(),
            address.function(),
            offset as u8,
        )
    }
    fn read_pci_u16(&self, address: acpi::PciAddress, offset: u16) -> u16 {
        Pci::read_u16(
            address.bus(),
            address.device(),
            address.function(),
            offset as u8,
        )
    }
    fn read_pci_u32(&self, address: acpi::PciAddress, offset: u16) -> u32 {
        Pci::read_u32(
            address.bus(),
            address.device(),
            address.function(),
            offset as u8,
        )
    }

    fn write_pci_u8(&self, address: acpi::PciAddress, offset: u16, value: u8) {
        Pci::write_u8(
            address.bus(),
            address.device(),
            address.function(),
            offset as u8,
            value,
        );
    }
    fn write_pci_u16(&self, address: acpi::PciAddress, offset: u16, value: u16) {
        Pci::write_u16(
            address.bus(),
            address.device(),
            address.function(),
            offset as u8,
            value,
        );
    }
    fn write_pci_u32(&self, address: acpi::PciAddress, offset: u16, value: u32) {
        Pci::write_u32(
            address.bus(),
            address.device(),
            address.function(),
            offset as u8,
            value,
        );
    }

    // TODO: find this with HPET probably.
    fn nanos_since_boot(&self) -> u64 {
        0
    }

    fn stall(&self, microseconds: u64) {
        // Busy wait.
        let start = unsafe { rdtsc() };
        // TODO: actually calculate this.
        while unsafe { rdtsc() } - start < microseconds * 1000 {
            spin_loop();
        }
    }
    fn sleep(&self, milliseconds: u64) {
        // TODO: use an actual sleep function.
        self.stall(milliseconds * 1000);
    }

    // TODO: create some mutexes or whatever.
    fn create_mutex(&self) -> acpi::Handle {
        acpi::Handle(0)
    }
    fn acquire(&self, mutex: acpi::Handle, timeout: u16) -> Result<(), AmlError> {
        Err(AmlError::LibUnimplemented)
    }
    fn release(&self, mutex: acpi::Handle) {}
}

impl Clone for KernelAcpiHandler {
    fn clone(&self) -> Self {
        Self {
        }
    }
}

pub struct AcpiInfo {
    pub tables: AcpiTables<KernelAcpiHandler>,
}

static ACPI: Once<AcpiInfo> = Once::new();

pub fn init_acpi(rsdp_address: usize) {
    kprintln!(
        "Initializing ACPI with RSDP at {:#x}.",
        rsdp_address,
    );
    let tables = unsafe {
        AcpiTables::from_rsdp(KernelAcpiHandler {}, rsdp_address)
            .expect("Failed to parse ACPI tables from RSDP.")
    };
    ACPI.call_once(|| AcpiInfo { tables });
    kprintln!("ACPI initialized.");
}

pub fn get_acpi() -> &'static AcpiInfo {
    ACPI.get().expect("ACPI not initialized.")
}

pub fn acpi_tables() {
    let acpi_info = get_acpi();
    let madt = acpi_info
        .tables
        .find_table::<Madt>()
        .expect("Failed to find MADT.");
    let hpet = acpi_info
        .tables
        .find_table::<HpetTable>()
        .expect("Failed to find HPET.");

    // TODO: probably do something with this information.
    dump_acpi_info(acpi_info, madt.get(), hpet.get());
}

fn dump_acpi_info(acpi_info: &AcpiInfo, madt: Pin<&Madt>, hpet: Pin<&HpetTable>) {
    // MADT.
    let local_apic_address = madt.local_apic_address;
    kprintln!("LAPIC address: {:#x}", local_apic_address);
    for entry in madt.entries() {
        match entry {
            MadtEntry::LocalApic(local_apic) => {
                let local_apic_flags = local_apic.flags;
                kprintln!(
                    "Local APIC: processor_id={}, apic_id={}, flags={:#x}",
                    local_apic.processor_id,
                    local_apic.apic_id,
                    local_apic_flags
                );
            }
            MadtEntry::IoApic(io_apic) => {
                let io_apic_address = io_apic.io_apic_address;
                let global_system_interrupt_base = io_apic.global_system_interrupt_base;
                kprintln!(
                    "IO APIC: id={}, address={:#x}, global_system_interrupt_base={}",
                    io_apic.io_apic_id,
                    io_apic_address,
                    global_system_interrupt_base
                );
            }
            MadtEntry::InterruptSourceOverride(iso) => {
                let global_system_interrupt = iso.global_system_interrupt;
                let flags = iso.flags;
                kprintln!(
                    "Interrupt Source Override: bus={}, source={}, global_system_interrupt={}, flags={:#x}",
                    iso.bus,
                    iso.irq,
                    global_system_interrupt,
                    flags
                );
            }
            MadtEntry::NmiSource(nmi_source) => {
                let nmi_source_flags = nmi_source.flags;
                let global_system_interrupt = nmi_source.global_system_interrupt;
                kprintln!(
                    "NMI Source: flags={:#x}, global_system_interrupt={}",
                    nmi_source_flags,
                    global_system_interrupt
                );
            }
            MadtEntry::LocalApicNmi(local_apic_nmi) => {
                let local_apic_nmi_flags = local_apic_nmi.flags;
                kprintln!(
                    "Local APIC NMI: processor_id={}, flags={:#x}, nmi_line={}",
                    local_apic_nmi.processor_id,
                    local_apic_nmi_flags,
                    local_apic_nmi.nmi_line
                );
            }
            MadtEntry::LocalApicAddressOverride(local_apic_address_override) => {
                let local_apic_address = local_apic_address_override.local_apic_address;
                kprintln!(
                    "Local APIC Address Override: local_apic_address={:#x}",
                    local_apic_address
                );
            }
            MadtEntry::IoSapic(io_sapic) => {
                let io_sapic_address = io_sapic.io_sapic_address;
                let global_system_interrupt_base = io_sapic.global_system_interrupt_base;
                kprintln!(
                    "IO SAPIC: id={}, address={:#x}, global_system_interrupt_base={}",
                    io_sapic.io_apic_id,
                    io_sapic_address,
                    global_system_interrupt_base
                );
            }
            MadtEntry::LocalSapic(local_sapic) => {
                kprintln!("Local SAPIC.");
            }
            MadtEntry::PlatformInterruptSource(platform_interrupt_source) => {
                kprintln!("Platform Interrupt Source.");
            }
            MadtEntry::LocalX2Apic(local_x2apic) => {
                let local_x2apic_id = local_x2apic.x2apic_id;
                let local_x2apic_flags = local_x2apic.flags;
                let local_x2apic_processor_uid = local_x2apic.processor_uid;
                kprintln!(
                    "Local x2APIC: id={}, flags={:#x}, processor_uid={}",
                    local_x2apic_id,
                    local_x2apic_flags,
                    local_x2apic_processor_uid
                );
            }
            MadtEntry::X2ApicNmi(x2apic_nmi) => {
                let x2apic_nmi_flags = x2apic_nmi.flags;
                let x2apic_nmi_processor_uid = x2apic_nmi.processor_uid;
                kprintln!(
                    "x2APIC NMI: flags={:#x}, processor_uid={}, nmi_line={}",
                    x2apic_nmi_flags,
                    x2apic_nmi_processor_uid,
                    x2apic_nmi.nmi_line
                );
            }
            MadtEntry::Gicc(gicc) => {
                kprintln!("GICC.");
            }
            MadtEntry::Gicd(gicd) => {
                kprintln!("GICD.");
            }
            MadtEntry::GicMsiFrame(gic_msi_frame) => {
                kprintln!("GIC MSI Frame.");
            }
            MadtEntry::GicRedistributor(gic_redistributor) => {
                kprintln!("GIC Redistributor.");
            }
            MadtEntry::GicInterruptTranslationService(gic_interrupt_translation_service) => {
                kprintln!("GIC Interrupt Translation Service.");
            }
            MadtEntry::MultiprocessorWakeup(multiprocessor_wakeup) => {
                kprintln!("Multiprocessor Wakeup.");
            }
        }
    }

    // HPET.
    let event_timer_block_id = hpet.event_timer_block_id;
    let base_address = hpet.base_address.address;
    let clock_tick_unit = hpet.clock_tick_unit;
    kprintln!(
        "HPET: event_timer_block_id={}, base_address={:#x}, hpet_number={}, clock_tick_unit={:#x}",
        event_timer_block_id,
        base_address,
        hpet.hpet_number,
        clock_tick_unit
    );
    let hpet_info = HpetInfo::new(&acpi_info.tables).expect("Failed to parse HPET info.");
    kprintln!(
        "HPET info: hardware_rev={}, num_comparators={}, main_counter_is_64bits={}, pci_vendor_id={:#x}, base_address={:#x}, hpet_number={}, clock_tick_unit={:#x}",
        hpet_info.hardware_rev,
        hpet_info.num_comparators,
        hpet_info.main_counter_is_64bits,
        hpet_info.pci_vendor_id,
        hpet_info.base_address,
        hpet_info.hpet_number,
        hpet_info.clock_tick_unit
    );

    // MCFG.
    if let Some(mcfg) = acpi_info.tables.find_table::<Mcfg>() {
        kprintln!("Found MCFG table.");
        for entry in mcfg.entries() {
            let base_address = entry.base_address;
            let segment_group = entry.pci_segment_group;
            kprintln!(
                "MCFG entry: base={:#x}, segment_group={:#x}, bus_start={:#x}, bus_end={:#x}",
                base_address,
                segment_group,
                entry.bus_number_start,
                entry.bus_number_end
            );
        }
    } else {
        kprintln!("No MCFG table found.");
    }
}
