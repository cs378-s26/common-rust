use alloc::vec::Vec;

use derive_more::Debug;
use limine::request::RsdpRequest;
use alloc::collections::BTreeMap;
use crate::sync::{IntSpinLock, MutexLike};
use crate::print::kprintln;

use crate::{
    devices::discovery::{DeviceNode, DeviceType, SYSTEM_DRIVERS, pcie::init_pcie},
    memory::{dma::MmioRegion, physical_memory::HHDM_OFFSET},
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
        flags: u16
    },
    Other(usize), // for now we only care about IO APIC entries, so we'll give a ptr to the rest
}

#[derive(Debug, Clone, Copy)]
pub enum TriggerMode {
    Edge,
    Level
}

#[derive(Debug, Clone, Copy)]
pub enum Polarity {
    ActiveHigh,
    ActiveLow
}

#[derive(Debug, Clone, Copy)]
pub struct InterruptOverride {
    pub bus_src: u8,
    pub irq_src: u8,
    pub gsi: u32,
    pub trigger_mode: TriggerMode,
    pub polarity: Polarity
}

// TODO: Construct table from isa irq num --> GSI from MADT entries

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
                        flags: u16
                    }
                    let override_ = unsafe { *(entry as *const SourceOverrideEntry) };
                    MadtEntry::ISOverride {
                        bus_src: override_.bus_src,
                        irq_src: override_.irq_src,
                        gsi: override_.gsi,
                        flags: override_.flags
                    }
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

static IOAPIC_OVERRIDE_GSI_MAP : IntSpinLock<BTreeMap<u8, InterruptOverride>> = IntSpinLock::new(BTreeMap::new()); // maps ISA IRQ num to GSI

pub fn get_gsi_for_irq(irq_num : u8) -> InterruptOverride {
    if let Some(gsi) = IOAPIC_OVERRIDE_GSI_MAP.lock().get(&irq_num) {
        *gsi
    } else {
        InterruptOverride {
            bus_src: 0,
            irq_src: irq_num,
            gsi: irq_num as u32,
            trigger_mode: TriggerMode::Edge,
            polarity: Polarity::ActiveHigh
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
    let mut matched_devices = Vec::new();

    for driver in SYSTEM_DRIVERS.iter() {
        // Walk the Madt and see if anyone wants to claim any of the entries
        for entry in madt.iterate_entries() {
            if let MadtEntry::ISOverride { bus_src, irq_src, gsi, flags } = entry {
                //kprintln!("[ACPI] Found interrupt source override: bus_src={}, irq_src={}, gsi={}, flags={:#x}", bus_src, irq_src, gsi, flags);
                IOAPIC_OVERRIDE_GSI_MAP.lock().insert(irq_src, 
                InterruptOverride {
                    bus_src,
                    irq_src,
                    gsi,
                    //Flags taken from https://wiki.osdev.org/MADT
                    //We assume Edge triggered and Active High polarity, per 
                    //https://wiki.osdev.org/IOAPIC
                    trigger_mode: if flags & 0b11 == 0b11 { TriggerMode::Level } else { TriggerMode::Edge },
                    polarity: if flags & 0b1000 != 0 { Polarity::ActiveLow } else { Polarity::ActiveHigh }
                }
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

    Some(matched_devices)
}
