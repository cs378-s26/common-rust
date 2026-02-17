// heap
// TODO: use virtual memory herez
pub static mut THE_HEAP: [u8; 256 * 1024 * 1024] = [0; _];
use crate::print::kprintln;

use limine::memory_map::{Entry, EntryType};
use limine::request::{MemoryMapRequest, HhdmRequest};
use core::mem::drop;
use spin::Mutex; // operations are quite short
use crate::arch::PAGE_SIZE;

// the below Limine-related code is partially from ChatGPT

#[unsafe(link_section = ".limine_requests")]
static MEMMAP_REQUEST: MemoryMapRequest = MemoryMapRequest::new();

#[unsafe(link_section = ".limine_requests")]
static HHDM_REQUEST: HhdmRequest = HhdmRequest::new();

struct FrameLocation {
    region : usize,
    offset : u64,
}

// TODO per-core cached lists of some sort?
static HEADLOCK: Mutex<u64> = Mutex::new(u64::MAX);
static ENDLOCK: Mutex<FrameLocation> = Mutex::new(FrameLocation{region: 0, offset: 0 as u64});

pub fn frame_alloc() -> u64 {
    let mut HEAD = HEADLOCK.lock();
    if *HEAD == u64::MAX {
        drop(HEAD); // not using this anymore
        let res = MEMMAP_REQUEST.get_response().unwrap(); // TODO cache get_response
        let mut END = ENDLOCK.lock();
        while END.offset + PAGE_SIZE as u64 > res.entries()[END.region].length {
            for region in (END.region+1)..res.entries().len() {
                if res.entries()[region].entry_type == EntryType::USABLE {
                    *END = FrameLocation{region: region, offset: 0};
                    break;
                }
                return frame_alloc(); // god-awful way to retry in case someone freed a frame
            }
        }
        let frame : u64 = HHDM_REQUEST.get_response().unwrap().offset() + res.entries()[END.region].base + END.offset;
        END.offset += PAGE_SIZE as u64;
        frame
    } else {
        let first : u64 = *HEAD;
        *HEAD = unsafe {*(first as *const u64)};
        first
    }
}

pub fn frame_dealloc(frame: u64) {
    let mut HEAD = HEADLOCK.lock();
    unsafe {
        *(frame as *mut u64) = *HEAD;
    }
    *HEAD = frame;
}