use alloc::{collections::BTreeMap, vec::Vec};

use derive_more::Debug;
use limine::request::RsdpRequest;

use crate::{
    devices::discovery::{DeviceNode, DeviceType, SYSTEM_DRIVERS, pcie::init_pcie},
    memory::{dma::MmioRegion, physical_memory::HHDM_OFFSET},
    print::kprintln,
    sync::{IntSpinLock, MutexLike},
};
// use virtio_drivers::read_config;

#[used]
#[unsafe(link_section = ".limine_requests")]
static RSDP_REQUEST: RsdpRequest = RsdpRequest::new();

fn physical_to_virtual(addr: usize) -> usize {
    addr + *HHDM_OFFSET
        .get()
        .expect("ACPI parsing attempted before HHDM_OFFSET was initialized")
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct Rsdp {
    signature: [u8; 8],
    checksum: u8,
    oem_id: [u8; 6],
    revision: u8,
    rsdt_address: u32,
    length: u32,
    xsdt_address: u64,
    extended_checksum: u8,
    reserved: [u8; 3],
}

impl Rsdp {
    fn from_address(addr: usize) -> Option<Self> {
        let rsdp = unsafe { *(addr as *const Rsdp) };
        if rsdp.revision < 2 {
            return None;
        }
        // validate signature and checksum
        if &rsdp.signature != b"RSD PTR " {
            return None;
        }
        let bytes = unsafe { core::slice::from_raw_parts(addr as *const u8, rsdp.length as usize) };
        let checksum: u8 = bytes.iter().fold(0u8, |acc, &b| acc.wrapping_add(b));
        if checksum != 0 {
            return None;
        }
        Some(rsdp)
    }

    fn get_xsdt(&self) -> Option<Xsdt> {
        Xsdt::from_address(physical_to_virtual(self.xsdt_address as usize))
    }
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct SDTHeader {
    signature: [u8; 4],
    length: u32,
    revision: u8,
    checksum: u8,
    oem_id: [u8; 6],
    oem_table_id: [u8; 8],
    oem_revision: u32,
    creator_id: u32,
    creator_revision: u32,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct Madt {
    header: SDTHeader,
    local_apic_address: u32,
    flags: u32,
    entries: usize,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct MadtEntryHeader {
    entry_type: u8,
    record_length: u8,
}

#[derive(Debug, Clone, Copy)]
pub enum MadtEntry {
    IOApic {
        id: u8,
        reserved: u8,
        address: u32,
        global_system_interrupt_base: u32,
    },
    ISOverride {
        bus_src: u8,
        irq_src: u8,
        gsi: u32,
        flags: u16,
    },
    Other(usize), // for now we only care about IO APIC entries, so we'll give a ptr to the rest
}

#[derive(Debug, Clone, Copy)]
pub enum TriggerMode {
    Edge,
    Level,
}

#[derive(Debug, Clone, Copy)]
pub enum Polarity {
    ActiveHigh,
    ActiveLow,
}

#[derive(Debug, Clone, Copy)]
pub struct InterruptOverride {
    pub bus_src: u8,
    pub irq_src: u8,
    pub gsi: u32,
    pub trigger_mode: TriggerMode,
    pub polarity: Polarity,
}

/*
* The LAPIC is not initialized when we do device discovery. Instead, get CPU -> LAPIC ID mapping 
from the MADT
*/
pub static IOAPIC_CPU_TO_LAPIC : IntSpinLock<BTreeMap<u32, u32>> = IntSpinLock::new(BTreeMap::new()); 


impl Madt {
    fn from_addr(addr: usize) -> Option<Self> {
        let mut madt = unsafe { *(addr as *mut Madt) };
        // checksum validation
        let bytes =
            unsafe { core::slice::from_raw_parts(addr as *const u8, madt.header.length as usize) };
        let checksum: u8 = bytes.iter().fold(0u8, |acc, &b| acc.wrapping_add(b));
        if &madt.header.signature != b"APIC" {
            return None;
        }
        if checksum != 0 {
            return None;
        }
        madt.entries = addr + 0x2C;
        Some(madt)
    }

    fn iterate_entries(&self) -> impl Iterator<Item = MadtEntry> {
        let mut current = self.entries;
        let end = self.entries + (self.header.length as usize - 0x2C);

        core::iter::from_fn(move || {
            if current >= end {
                return None;
            }

            let entry = current as *const MadtEntryHeader;
            let length = unsafe { (*entry).record_length as usize };
            if length < core::mem::size_of::<MadtEntryHeader>() || current + length > end {
                return None;
            }

            current += length;
            Some(match unsafe { (*entry).entry_type } {
                0 => {
                    #[repr(C, packed)]
                    #[derive(Clone, Copy)]
                    struct LapicEntry {
                        header: MadtEntryHeader,
                        processor_id: u8,
                        lapic_id: u8,
                        flags: u32,
                    }
                    let lapic = unsafe { *(entry as *const LapicEntry) };
                    IOAPIC_CPU_TO_LAPIC
                        .lock()
                        .insert(lapic.processor_id as u32, lapic.lapic_id as u32);
                    MadtEntry::Other(entry as usize) // we don't actually care about LAPIC entries for device discovery, so we'll just give a ptr to it
                }
                1 => {
                    #[repr(C, packed)]
                    #[derive(Clone, Copy)]
                    struct IoApicEntry {
                        header: MadtEntryHeader,
                        id: u8,
                        reserved: u8,
                        address: u32,
                        global_system_interrupt_base: u32,
                    }

                    let ioapic = unsafe { *(entry as *const IoApicEntry) };
                    MadtEntry::IOApic {
                        id: ioapic.id,
                        reserved: ioapic.reserved,
                        address: ioapic.address,
                        global_system_interrupt_base: ioapic.global_system_interrupt_base,
                    }
                }
                2 => {
                    #[repr(C, packed)]
                    #[derive(Clone, Copy)]
                    struct SourceOverrideEntry {
                        header: MadtEntryHeader,
                        bus_src: u8,
                        irq_src: u8,
                        gsi: u32,
                        flags: u16,
                    }
                    let override_ = unsafe { *(entry as *const SourceOverrideEntry) };
                    MadtEntry::ISOverride {
                        bus_src: override_.bus_src,
                        irq_src: override_.irq_src,
                        gsi: override_.gsi,
                        flags: override_.flags,
                    }
                }
                9 => {
                    #[repr(C, packed)]
                    #[derive(Clone, Copy)]
                    struct LapicX2Entry {
                        header: MadtEntryHeader,
                        reserved: u16,
                        processor_id: u32,
                        lapic_id: u32,
                        flags: u32,
                    }
                    let lapicx2 = unsafe { *(entry as *const LapicX2Entry) };
                    IOAPIC_CPU_TO_LAPIC
                        .lock()
                        .insert(lapicx2.processor_id, lapicx2.lapic_id);
                    MadtEntry::Other(entry as usize) // we don't actually care about LAPIC entries for device discovery, so we'll just give a ptr to it
                }
                _ => MadtEntry::Other(entry as usize),
            })
        })
    }
}

#[derive(Clone)]
pub struct Mcfg {
    header: SDTHeader,
    entries: Vec<McfgEntry>,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct McfgEntry {
    pub base_address: u64,
    pub segment_group_number: u16,
    pub start_bus_number: u8,
    pub end_bus_number: u8,
    pub reserved: [u8; 4],
}

impl Mcfg {
    fn from_addr(addr: usize) -> Option<Self> {
        let header = unsafe { *(addr as *const SDTHeader) };
        let mut mcfg = Mcfg {
            header,
            entries: Vec::new(),
        };
        // checksum validation
        let bytes =
            unsafe { core::slice::from_raw_parts(addr as *const u8, mcfg.header.length as usize) };
        let checksum: u8 = bytes.iter().fold(0u8, |acc, &b| acc.wrapping_add(b));
        if &mcfg.header.signature != b"MCFG" {
            return None;
        }
        if checksum != 0 {
            return None;
        }
        let mut current = addr + core::mem::size_of::<SDTHeader>() + 8; // header + reserved
        let end = addr + mcfg.header.length as usize;
        while current + core::mem::size_of::<McfgEntry>() <= end {
            let mut entry = unsafe { *(current as *const McfgEntry) };
            // map the physical address to virtual for later
            let num_buses = (entry.end_bus_number - entry.start_bus_number) as usize + 1;
            let mapping = MmioRegion::new(entry.base_address as usize, num_buses * 32 * 8 * 4096);
            entry.base_address = mapping.virt_addr() as u64;
            mcfg.entries.push(entry);
            core::mem::forget(mapping); // We don't want to drop the mapping
            current += core::mem::size_of::<McfgEntry>();
        }
        Some(mcfg)
    }

    pub fn iterate_entries(&self) -> impl Iterator<Item = &McfgEntry> {
        self.entries.iter()
    }
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct FADT {
    //Copilot copied off of https://elixir.bootlin.com/linux/v7.0.1/source/include/acpi/actbl.h#L236
    sdtheader : SDTHeader,
    facs : u32,
    dsdt : u32,
    reserved : u8,
    preferred_pm_profile : u8,
    sci_interrupt : u16,
    smi_command_port : u32,
    acpi_enable : u8,
    acpi_disable : u8,
    s4bios_req : u8,
    pstate_control : u8,
    pm1a_event_block : u32,
    pm1b_event_block : u32,
    pm1a_control_block : u32,
    pm1b_control_block : u32,
    pm2_control_block : u32,
    pm_timer_block : u32,
    gpe0_block : u32,
    gpe1_block : u32,
    pm1_event_length : u8,
    pm1_control_length : u8,
    pm2_control_length : u8,
    pm_timer_length : u8,
    gpe0_length : u8,
    gpe1_length : u8,
    gpe1_base : u8,
    cst_control : u8,
    c2_latency : u16,
    c3_latency : u16,
    flush_size : u16,
    flush_stride : u16,
    duty_offset : u8,
    duty_width : u8,
    day_alarm : u8,
    month_alarm : u8,
    year_alarm : u8,
    flags : u32,
}

impl FADT {
    fn from_addr(addr: usize) -> Option<Self> {
        let fadt = unsafe { *(addr as *const FADT) };
        if &fadt.sdtheader.signature != b"FACP" {
            return None;
        }
        // checksum validation
        let bytes =
            unsafe { core::slice::from_raw_parts(addr as *const u8, fadt.sdtheader.length as usize) };
        let checksum: u8 = bytes.iter().fold(0u8, |acc, &b| acc.wrapping_add(b));
        if checksum != 0 {
            return None;
        }
        Some(fadt)
    }
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct Xsdt {
    header: SDTHeader,
    sdt_ptrs: usize,
}

impl Xsdt {
    fn from_address(addr: usize) -> Option<Self> {
        let mut xsdt = unsafe { *(addr as *mut Xsdt) };
        // checksum validation
        let bytes =
            unsafe { core::slice::from_raw_parts(addr as *const u8, xsdt.header.length as usize) };
        let checksum: u8 = bytes.iter().fold(0u8, |acc, &b| acc.wrapping_add(b));
        if checksum != 0 {
            return None;
        }
        xsdt.sdt_ptrs = addr + core::mem::size_of::<SDTHeader>();
        Some(xsdt)
    }

    fn get_ptrs(&self) -> impl Iterator<Item = usize> + '_ {
        let bytes_len = self.header.length as usize - core::mem::size_of::<SDTHeader>();
        let bytes = unsafe { core::slice::from_raw_parts(self.sdt_ptrs as *const u8, bytes_len) };

        bytes.chunks_exact(8).map(|chunk| {
            let addr = u64::from_le_bytes(chunk.try_into().unwrap());
            addr as usize
        })
    }
    fn parse_fadt(&self) -> Option<FADT> {
        for ptr in self.get_ptrs() {
            let fadt = FADT::from_addr(physical_to_virtual(ptr));
            if fadt.is_some() {
                return fadt;
            }
        }
        None
    }

    fn parse_madt(&self) -> Option<Madt> {
        for ptr in self.get_ptrs() {
            let madt = Madt::from_addr(physical_to_virtual(ptr));
            if madt.is_some() {
                return madt;
            }
        }
        None
    }

    fn parse_mcfg(&self) -> Option<Mcfg> {
        for ptr in self.get_ptrs() {
            let mcfg = Mcfg::from_addr(physical_to_virtual(ptr));
            if mcfg.is_some() {
                return mcfg;
            }
        }
        None
    }
}

/*
* List of overrides from ISQ IRQs to GSIs. 
*/
static IOAPIC_OVERRIDE_GSI_MAP: IntSpinLock<BTreeMap<u8, InterruptOverride>> =
    IntSpinLock::new(BTreeMap::new()); // maps ISA IRQ num to GSI

/*
* Given an ISA IRQ number, return the GSI Number. Primarily used for programming
the I/O APIC, though it might be useful for the GIC too. If there is no override,
we assume the GSI is the same as the IRQ number, and that it's edge triggered and active high, per the ACPI spec.
*/
pub fn get_gsi_for_irq(irq_num: u8) -> InterruptOverride {
    if let Some(gsi) = IOAPIC_OVERRIDE_GSI_MAP.lock().get(&irq_num) {
        *gsi
    } else {
        InterruptOverride {
            bus_src: 0,
            irq_src: irq_num,
            gsi: irq_num as u32,
            trigger_mode: TriggerMode::Edge,
            polarity: Polarity::ActiveHigh,
        }
    }
}

// Currently ACPI is in this weird spot
// I'm trying my hardest to avoid creating an AML interpreter
// So basically this things' job is only to parse
// IO-APIC/GIC + PCI via mcfg
// To set up the IO-APIC w/ PCI we _need_ an AML interpreter, so we're just going to ...not
pub fn parse_acpi() -> Option<Vec<DeviceType>> {
    let rsdp_ptr = RSDP_REQUEST.get_response()?.address();
    let xsdt = Rsdp::from_address(rsdp_ptr)?.get_xsdt()?;
    let madt = xsdt.parse_madt()?;
    let mcfg = xsdt.parse_mcfg()?;
    let fadt_ = xsdt.parse_fadt();
    let mut matched_devices = Vec::new();

    for driver in SYSTEM_DRIVERS.iter() {
        // Walk the Madt and see if anyone wants to claim any of the entries
        for entry in madt.iterate_entries() {
            if let MadtEntry::ISOverride {
                bus_src,
                irq_src,
                gsi,
                flags,
            } = entry
            {
                kprintln!("[ACPI] Found interrupt source override: bus_src={}, irq_src={}, gsi={}, flags={:#x}", bus_src, irq_src, gsi, flags);
                IOAPIC_OVERRIDE_GSI_MAP.lock().insert(
                    irq_src,
                    InterruptOverride {
                        bus_src,
                        irq_src,
                        gsi,
                        //Flags taken from https://wiki.osdev.org/MADT
                        //We assume Edge triggered and Active High polarity, per
                        //https://wiki.osdev.org/IOAPIC
                        trigger_mode: if flags & 0b11 == 0b11 {
                            TriggerMode::Level
                        } else {
                            TriggerMode::Edge
                        },
                        polarity: if flags & 0b1000 != 0 {
                            Polarity::ActiveLow
                        } else {
                            Polarity::ActiveHigh
                        },
                    },
                );
            }
            let device = driver.am_i_this(DeviceNode::MadtEntry(entry));
            if let Some(d) = device {
                matched_devices.extend(d);
                break;
            }
        }
    }
    // Parse PCI-E w/ the MCFG
    matched_devices.extend(init_pcie(mcfg.clone()));

    if let Some(fadt) = fadt_ {
        //source: https://elixir.bootlin.com/linux/v7.0.1/source/include/acpi/actbl.h#L261
        let ps2_enabled = fadt.flags & (1 << 1) != 0;
        //kprintln!("PS2 enabled? {}", ps2_enabled);
        if ps2_enabled {
            crate::devices::char::ps2_kb_m::init_ps2().ok()?;
            //matched_devices.push(DeviceType::Char(crate::devices::char::ps2_kb_m::init_ps2));
        }
    }
    Some(matched_devices)
}
