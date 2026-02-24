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
static HEAD: Mutex<usize> = Mutex::new(usize::MAX);
static END: Mutex<FrameLocation> = Mutex::new(FrameLocation{region: 0, offset: 0});

pub fn frame_alloc() -> usize {
    let mut head = HEAD.lock();
    if *head == usize::MAX {
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
        let frame : usize = res.entries()[end.region].base as usize + end.offset;
        end.offset += PAGE_SIZE;
        frame
    } else {
        let first : usize = *head;
        *head = unsafe {*((HHDM_REQUEST.get_response().unwrap().offset() as usize + first) as *const usize)};
        first
    }
}

pub fn frame_dealloc(frame: usize) {
    let mut head = HEAD.lock();
    unsafe {
        *((HHDM_REQUEST.get_response().unwrap().offset() as usize + frame) as *mut usize) = *head;
    }
    *head = frame;
}