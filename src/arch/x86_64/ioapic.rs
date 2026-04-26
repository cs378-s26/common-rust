use alloc::vec;

use spin::Once;

use crate::{
    devices::discovery::{
        DeviceDiscovery,
        acpi::{MadtEntry, Polarity, TriggerMode},
    },
    memory::dma::MmioRegion,
    print::kprintln,
};

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

// Keeps the MmioRegion alive so the MMIO page is never unmapped.
static IOAPIC: Once<MmioRegion> = Once::new();

fn base() -> *mut u32 {
    IOAPIC.get().expect("IOAPIC not initialized").virt_addr() as *mut u32
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
    IOAPIC.call_once(|| MmioRegion::new(phys_base as usize, 4096));

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
pub fn route_irq(
    irq: u8,
    vector: u8,
    dest_apic_id: u32,
    trigger_mode: TriggerMode,
    polarity: Polarity,
) {
    let mut cur_lo: u32 = vector as u32;
    match trigger_mode {
        // see https://wiki.osdev.org/IOAPIC#IOREGSEL_and_IOWIN
        TriggerMode::Edge => cur_lo &= !(1 << 15), // clear bit 15 for edge-triggered
        TriggerMode::Level => cur_lo |= 1 << 15,   // set bit 15 for level-triggered
    }
    match polarity {
        Polarity::ActiveHigh => cur_lo &= !(1 << 13), // clear bit 13 for active-high
        Polarity::ActiveLow => cur_lo |= 1 << 13,     // set bit 13 for active-low
    }
    let hi: u32 = (dest_apic_id & 0xFF) << 24; // destination APIC ID in bits[31:24]

    unsafe {
        write_reg(redir_hi(irq), hi);
        write_reg(redir_lo(irq), cur_lo); // write lo last — unmasks the entry
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

pub struct Discovery {}
impl DeviceDiscovery for Discovery {
    fn name(&self) -> &'static str {
        "x86_64 IOAPIC discovery"
    }
    fn am_i_this(
        &self,
        node: crate::devices::discovery::DeviceNode,
    ) -> Option<alloc::vec::Vec<crate::devices::discovery::DeviceType>> {
        if let crate::devices::discovery::DeviceNode::MadtEntry(MadtEntry::IOApic {
            id,
            address,
            ..
        }) = node
        {
            kprintln!(
                "[IOAPIC discovery] found IOAPIC with id={} addr={:#x}",
                id,
                address
            );
            init_ioapic(address as u64);
            Some(vec![crate::devices::discovery::DeviceType::Special])
        } else {
            None
        }
    }

    fn run_at_start(&self) -> bool {
        true
    }
}
