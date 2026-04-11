// heap
// TODO: use virtual memory herez
pub static mut THE_HEAP: [u8; 256 * 1024 * 1024] = [0; _];

use crate::{
    arch::{Arch, ArchTrait},
    kprintln,
};
use alloc::string::{String, ToString};
use core::{mem::drop, ptr};
use limine::memory_map::{Entry, EntryType};
use limine::request::{HhdmRequest, MemoryMapRequest};
use spin::{Mutex, Once}; // operations are quite short

// the below Limine-related code is partially from ChatGPT

#[unsafe(link_section = ".limine_requests")]
static MEMMAP_REQUEST: MemoryMapRequest = MemoryMapRequest::new();

#[unsafe(link_section = ".limine_requests")]
pub static HHDM_REQUEST: HhdmRequest = HhdmRequest::new();

struct FrameLocation {
    region: usize,
    offset: usize,
}

// TODO add per-core cached lists of some sort?
static HEAD: Mutex<usize> = Mutex::new(usize::MAX);
static END: Mutex<FrameLocation> = Mutex::new(FrameLocation {
    region: 0,
    offset: 0,
});

pub static REGIONS: Once<&[&Entry]> = Once::new();
pub static HHDM_OFFSET: Once<usize> = Once::new();

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
        _ => panic!("Unexpected Limine memory map entry type"),
    }
    .to_string()
}

pub fn init_physical_memory_allocator() {
    HHDM_OFFSET.call_once(|| HHDM_REQUEST.get_response().unwrap().offset() as usize);
    REGIONS.call_once(|| {
        let entries = MEMMAP_REQUEST.get_response().unwrap().entries();
        kprintln!("\nLimine Memory Map:");
        for entry in entries {
            kprintln!(
                "{:016x}-{:016x} ({})",
                entry.base,
                entry.base + entry.length,
                display_entry_type(entry.entry_type)
            );
        }
        entries
    });

    let mut end = END.lock(); // the first memory region served wasn't checked to be usable
    let regions = unwrap(&REGIONS);
    if let Some(first_usable) =
        (0..regions.len()).find(|&r| regions[r].entry_type == EntryType::USABLE)
    {
        *end = FrameLocation {
            region: first_usable,
            offset: 0x0,
        };
    } else {
        panic!("No usable memory regions found");
    }
}

fn unwrap<T>(o: &Once<T>) -> &T {
    o.get().unwrap()
}

pub fn frame_alloc() -> usize {
    let mut head = HEAD.lock();
    if *head == usize::MAX {
        drop(head); // not using this anymore
        let mut end = END.lock();

        let regions = unwrap(&REGIONS);

        while end.offset + Arch::PAGE_SIZE > regions[end.region].length as usize {
            // try to find the next usable region
            if let Some(region) = ((end.region + 1)..regions.len())
                .find(|&r| regions[r].entry_type == EntryType::USABLE)
            {
                *end = FrameLocation { region, offset: 0 };
                continue;
            }

            // no usable region found - retry allocation
            drop(end);
            return frame_alloc(); // waits for a physical page to be freed
        }

        let entry = unwrap(&REGIONS)[end.region];

        let frame: usize = entry.base as usize + end.offset;
        end.offset += Arch::PAGE_SIZE;
        frame
    } else {
        let first: usize = *head;
        if !(first.is_multiple_of(Arch::PAGE_SIZE)) {
            panic!("attempted to allocate unaligned frame {:x}", first);
        }
        *head = unsafe { *((unwrap(&HHDM_OFFSET) + first) as *const usize) };
        first
    }
}

// maps 'frames' number of contiguous frames, used for DMA
// TODO we don't really want this to be a bump allocator we want to be able
// to better reclaim memory
pub fn alloc_frames(frames: usize) -> usize {
    assert!(frames > 0);

    let mut end = END.lock();
    let regions = unwrap(&REGIONS);
    while end.offset + Arch::PAGE_SIZE * frames > regions[end.region].length as usize {
        if let Some(region) =
            ((end.region + 1)..regions.len()).find(|&r| regions[r].entry_type == EntryType::USABLE)
        {
            *end = FrameLocation { region, offset: 0 };
            continue;
        }
        panic!("No usable memory regions found");
    }

    let entry = unwrap(&REGIONS)[end.region];
    let frame: usize = entry.base as usize + end.offset;
    end.offset += Arch::PAGE_SIZE * frames;
    frame
}

pub fn frame_dealloc(frame: usize) {
    assert!(frame.is_multiple_of(Arch::PAGE_SIZE));
    let mut head = HEAD.lock();
    unsafe {
        *((unwrap(&HHDM_OFFSET) + frame) as *mut usize) = *head;
    }
    *head = frame;
}

/// # Safety
///
/// This function treats src and dst as raw pointers to physical (not
/// virtual) memory. It is expected that src and dst are NOT already
/// adjusted by the HHDM offset, as this function does that for you.
pub unsafe fn copy(src: usize, dst: usize, length: usize) {
    let hhdm = HHDM_OFFSET.get().unwrap();
    unsafe { ptr::copy_nonoverlapping((src + hhdm) as *const u8, (dst + hhdm) as *mut u8, length) };
}
