/// Minimal ACPI MADT parser finds the IOAPIC base address and ISA IRQ source overrides.
use core::mem::size_of;

use limine::request::RsdpRequest;

use crate::physical_memory::phys_to_virt;

#[used]
#[unsafe(link_section = ".limine_requests")]
static RSDP_REQUEST: RsdpRequest = RsdpRequest::new();

/// Root System Description Pointer — the firmware entry point into ACPI, found via Limine.
#[repr(C, packed)]
struct Rsdp {
    signature: [u8; 8],
    checksum: u8,
    oem_id: [u8; 6],
    revision: u8,
    rsdt_addr: u32,
    length: u32,
    xsdt_addr: u64,
    ext_checksum: u8,
    _reserved: [u8; 3],
}

/// Common 36-byte header that begins every ACPI System Description Table.
#[repr(C, packed)]
struct SdtHeader {
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

const MADT_SIG: [u8; 4] = *b"APIC";

/// MADT-specific header follows SdtHeader and precedes the variable-length entry list.
#[repr(C, packed)]
struct MadtHeader {
    sdt: SdtHeader,
    local_apic_addr: u32,
    flags: u32,
}

/// MADT entry type 1: describes one IOAPIC and the GSI range it handles.
#[repr(C, packed)]
struct MadtIoApic {
    entry_type: u8, // 1
    length: u8,     // 12
    ioapic_id: u8,
    _reserved: u8,
    ioapic_addr: u32,
    gsi_base: u32,
}

/// MADT entry type 2: remaps a legacy ISA IRQ to a Global System Interrupt number.
#[repr(C, packed)]
struct MadtIrqOverride {
    entry_type: u8, // 2
    length: u8,     // 10
    bus: u8,        // 0 = ISA
    source: u8,     // ISA IRQ
    gsi: u32,       // mapped Global System Interrupt
    flags: u16,
}

/// Returns the physical base address of the first IOAPIC found in the MADT.
pub fn find_ioapic_base() -> Option<u64> {
    let madt = find_madt()?;
    iter_madt_entries(madt, |ptr, entry_type, _len| {
        if entry_type == 1 {
            let e = unsafe { &*(ptr as *const MadtIoApic) };
            return Some(e.ioapic_addr as u64);
        }
        None
    })
}

/// Maps an ISA IRQ number to its Global System Interrupt (GSI) via MADT overrides.
/// Falls back to identity mapping (isa_irq == gsi) if no override exists.
pub fn find_irq_override(isa_irq: u8) -> u8 {
    let Some(madt) = find_madt() else {
        return isa_irq;
    };
    iter_madt_entries(madt, |ptr, entry_type, _len| {
        if entry_type == 2 {
            let e = unsafe { &*(ptr as *const MadtIrqOverride) };
            if e.bus == 0 && e.source == isa_irq {
                return Some(e.gsi as u8);
            }
        }
        None
    })
    .unwrap_or(isa_irq)
}

/// Locate the MADT by walking RSDP to XSDT.
fn find_madt() -> Option<*const u8> {
    // Limine gives us a virtual address for the RSDP directly — don't apply HHDM to it.
    // Addresses *inside* the RSDP (xsdt_addr, rsdt_addr) are physical and need phys_to_virt.
    let rsdp = unsafe { &*(RSDP_REQUEST.get_response()?.address() as *const Rsdp) };

    if &rsdp.signature != b"RSD PTR " {
        return None;
    }

    if rsdp.revision >= 2 && rsdp.xsdt_addr != 0 {
        find_table_in_sdt(rsdp.xsdt_addr, MADT_SIG, 8)
    } else {
        find_table_in_sdt(rsdp.rsdt_addr as u64, MADT_SIG, 4)
    }
}

/// Search an SDT (XSDT or RSDT) for a table with the given signature.
/// `ptr_size` is 8 for XSDT (64-bit pointers) or 4 for RSDT (32-bit pointers).
fn find_table_in_sdt(sdt_phys: u64, sig: [u8; 4], ptr_size: usize) -> Option<*const u8> {
    let base = phys_to_virt(sdt_phys);
    let header = unsafe { &*(base as *const SdtHeader) };
    let entry_count = (header.length as usize - size_of::<SdtHeader>()) / ptr_size;
    let entries = unsafe { base.add(size_of::<SdtHeader>()) };

    for i in 0..entry_count {
        let entry_ptr = unsafe { entries.add(i * ptr_size) };
        let phys = if ptr_size == 8 {
            unsafe { (entry_ptr as *const u64).read_unaligned() }
        } else {
            unsafe { (entry_ptr as *const u32).read_unaligned() as u64 }
        };
        let table = phys_to_virt(phys);
        if unsafe { &*(table as *const SdtHeader) }.signature == sig {
            return Some(table);
        }
    }
    None
}

/// Walk every MADT entry; return the first `Some` value produced by `f`.
fn iter_madt_entries<T>(
    madt: *const u8,
    mut f: impl FnMut(*const u8, u8, u8) -> Option<T>,
) -> Option<T> {
    let header = unsafe { &*(madt as *const MadtHeader) };
    let total_len = header.sdt.length as usize;
    let entries_start = unsafe { madt.add(size_of::<MadtHeader>()) };
    let entries_end = unsafe { madt.add(total_len) };

    let mut ptr = entries_start;
    while ptr < entries_end {
        let entry_type = unsafe { *ptr };
        let length = unsafe { *ptr.add(1) };
        if length < 2 {
            break;
        }
        if let Some(v) = f(ptr, entry_type, length) {
            return Some(v);
        }
        ptr = unsafe { ptr.add(length as usize) };
    }
    None
}
