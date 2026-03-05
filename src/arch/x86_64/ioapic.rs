use spin::Once;

use crate::arch::{Arch, ArchTrait};
use crate::print::kprintln;
use crate::virtual_memory::{PagingOptions, VirtualMemoryAllocation};

const IOREGSEL: usize = 0x00;
const IOWIN: usize = 0x10;

const REG_ID: u8 = 0x00;
const REG_VER: u8 = 0x01;

fn redir_lo(irq: u8) -> u8 {
    0x10 + irq * 2
}
fn redir_hi(irq: u8) -> u8 {
    0x10 + irq * 2 + 1
}

// Keeps the VirtualMemoryAllocation alive so the MMIO page is never unmapped.
static IOAPIC: Once<VirtualMemoryAllocation> = Once::new();

fn base() -> *mut u32 {
    IOAPIC.get().expect("IOAPIC not initialized").base() as *mut u32
}

unsafe fn read_reg(reg: u8) -> u32 {
    let b = base();
    unsafe {
        b.byte_add(IOREGSEL).write_volatile(reg as u32);
        b.byte_add(IOWIN).read_volatile()
    }
}

unsafe fn write_reg(reg: u8, val: u32) {
    let b = base();
    unsafe {
        b.byte_add(IOREGSEL).write_volatile(reg as u32);
        b.byte_add(IOWIN).write_volatile(val);
    }
}

/// Initialize the IOAPIC. Must be called once on the BSP before routing IRQs.
/// `phys_base` is the IOAPIC physical address (from ACPI MADT or fallback 0xFEC00000).
///
/// The IOAPIC MMIO region is NOT in the HHDM (it's not RAM), so we allocate a virtual
/// page and map the physical MMIO page into it using the kernel's virtual memory allocator.
/// Note: PCD (cache-disable) is not yet wired through the VMM, so QEMU is fine but real
/// hardware would need write-combining or uncached mappings for correct MMIO behaviour.
pub fn init_ioapic(phys_base: u64) {
    IOAPIC.call_once(|| {
        VirtualMemoryAllocation::new(
            Arch::get_address_space(),
            4096,
            Some(phys_base as usize),
            PagingOptions::PRESENT | PagingOptions::WRITABLE,
        )
    });

    let ver = unsafe { read_reg(REG_VER) };
    let max_redir = ((ver >> 16) & 0xFF) as u8;
    let id = unsafe { read_reg(REG_ID) };

    kprintln!(
        "[IOAPIC] phys={:#x} virt={:p} id={} version={:#x} max_redir={}",
        phys_base,
        base(),
        (id >> 24) & 0xF,
        ver & 0xFF,
        max_redir
    );

    // Mask all redirection entries so we start with a clean slate.
    for irq in 0..=max_redir {
        mask_irq(irq);
    }
}

/// Route an IRQ line to a CPU vector, delivered to `dest_apic_id` (physical mode).
pub fn route_irq(irq: u8, vector: u8, dest_apic_id: u32) {
    let lo: u32 = vector as u32; // bits[16]=0 (unmasked), delivery=fixed
    let hi: u32 = (dest_apic_id & 0xFF) << 24; // destination APIC ID in bits[31:24]

    unsafe {
        write_reg(redir_hi(irq), hi);
        write_reg(redir_lo(irq), lo); // write lo last — unmasks the entry
    }

    let read_lo = unsafe { read_reg(redir_lo(irq)) };
    let read_hi = unsafe { read_reg(redir_hi(irq)) };
    kprintln!(
        "[IOAPIC] IRQ{} → vec={:#04x} dest_apic={} redir={:#010x}_{:#010x}",
        irq,
        vector,
        dest_apic_id,
        read_hi,
        read_lo
    );
}

/// Mask (disable) an IRQ line.
pub fn mask_irq(irq: u8) {
    let lo = unsafe { read_reg(redir_lo(irq)) };
    unsafe { write_reg(redir_lo(irq), lo | (1 << 16)) };
}
