// heap
// TODO: use virtual memory herez
pub static mut THE_HEAP: [u8; 256 * 1024 * 1024] = [0; _];

use core::mem::drop;
use spin::{Mutex, Once}; // operations are quite short
use alloc::string::{String, ToString};
use limine::memory_map::{Entry, EntryType};
use limine::request::{MemoryMapRequest, HhdmRequest};
use crate::arch::{Arch, ArchTrait};
use crate::print::kprintln;

// the below Limine-related code is partially from ChatGPT

#[unsafe(link_section = ".limine_requests")]
static MEMMAP_REQUEST: MemoryMapRequest = MemoryMapRequest::new();

#[unsafe(link_section = ".limine_requests")]
pub static HHDM_REQUEST: HhdmRequest = HhdmRequest::new();

struct FrameLocation {
    region : usize,
    offset : usize,
}

// TODO add per-core cached lists of some sort?
static HEAD: Mutex<usize> = Mutex::new(usize::MAX);
static END: Mutex<FrameLocation> = Mutex::new(FrameLocation{region: 0, offset: 0});

static REGIONS: Once<&[&Entry]> = Once::new();
static HHDM_OFFSET: Once<usize> = Once::new();

fn display_entry_type(et: EntryType) -> String {
    match et {
        EntryType::USABLE => "Usable",
        EntryType::RESERVED => "Reserved permanently",
        EntryType::ACPI_RECLAIMABLE => "Reclaimable from ACPI",
        EntryType::ACPI_NVS => "Reserved for ACPI",
        EntryType::BAD_MEMORY => "Unusable hardware",
        EntryType::BOOTLOADER_RECLAIMABLE => "Reclaimable from Limine",
        EntryType::EXECUTABLE_AND_MODULES => "Reserved for kernel code",
        EntryType::FRAMEBUFFER => "Reserved for frame buffer",
        _ => panic!("Unexpected Limine memory map entry type")
    }.to_string()
}

pub fn init_physical_memory_allocator() {
    HHDM_OFFSET.call_once(||
        HHDM_REQUEST.get_response().unwrap().offset() as usize
    );
    REGIONS.call_once(|| {
        let entries = MEMMAP_REQUEST.get_response().unwrap().entries();
        kprintln!("\nLimine Memory Map:");
        for entry in entries {
            kprintln!("{:016x}-{:016x} ({})", entry.base, entry.base + entry.length, display_entry_type(entry.entry_type));
        }
        kprintln!("");
        entries
    });
}

fn unwrap<T>(o: &Once<T>) -> &T {
    &o.get().expect("")
}

pub fn frame_alloc() -> usize {
    let mut head = HEAD.lock();
    if *head == usize::MAX {
        drop(head); // not using this anymore
        let mut end = END.lock();
        'outer: while end.offset + Arch::PAGE_SIZE > unwrap(&REGIONS)[end.region].length as usize {
            for region in (end.region+1)..unwrap(&REGIONS).len() {
                if unwrap(&REGIONS)[region].entry_type == EntryType::USABLE {
                    *end = FrameLocation{region: region, offset: 0};
                    break 'outer;
                }
            }
            drop(end);
            return frame_alloc(); // god-awful mechanism for waiting for a physical page to be freed
        }
        let frame : usize = unwrap(&REGIONS)[end.region].base as usize + end.offset;
        end.offset += Arch::PAGE_SIZE;
        frame
    } else {
        let first : usize = *head;
        *head = unsafe {*((unwrap(&HHDM_OFFSET) + first) as *const usize)};
        first
    }
}

pub fn frame_dealloc(frame: usize) {
    let mut head = HEAD.lock();
    unsafe {
        *((unwrap(&HHDM_OFFSET) + frame) as *mut usize) = *head;
    }
    *head = frame;
}