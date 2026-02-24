// heap
// TODO: use virtual memory herez
pub static mut THE_HEAP: [u8; 256 * 1024 * 1024] = [0; _];

use limine::memory_map::{Entry, EntryType};
use limine::request::{MemoryMapRequest, HhdmRequest};
use core::mem::drop;
use spin::Mutex; // operations are quite short
use crate::arch::PAGE_SIZE;

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
static HEAD: Mutex<u64> = Mutex::new(u64::MAX);
static END: Mutex<FrameLocation> = Mutex::new(FrameLocation{region: 0, offset: 0});

pub fn frame_alloc() -> u64 {
    let mut head = HEAD.lock();
    if *head == u64::MAX {
        drop(head); // not using this anymore
        let res = MEMMAP_REQUEST.get_response().unwrap(); // TODO cache get_response
        let mut end = END.lock();
        while end.offset + PAGE_SIZE > res.entries()[end.region].length as usize {
            for region in (end.region+1)..res.entries().len() {
                if res.entries()[region].entry_type == EntryType::USABLE {
                    *end = FrameLocation{region: region, offset: 0};
                    break;
                }
                return frame_alloc(); // god-awful mechanism for waiting for a physical page to be freed
            }
        }
        let frame : u64 = res.entries()[end.region].base + end.offset as u64;
        end.offset += PAGE_SIZE;
        frame
    } else {
        let first : u64 = *head;
        *head = unsafe {*((HHDM_REQUEST.get_response().unwrap().offset() + first) as *const u64)};
        first
    }
}

pub fn frame_dealloc(frame: u64) {
    let mut head = HEAD.lock();
    unsafe {
        *((HHDM_REQUEST.get_response().unwrap().offset() + frame) as *mut u64) = *head;
    }
    *head = frame;
}