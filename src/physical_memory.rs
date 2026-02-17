// heap
// TODO: use virtual memory herez
pub static mut THE_HEAP: [u8; 256 * 1024 * 1024] = [0; _];
use crate::sync::IntMutex;

use limine::memory_map::{Entry, EntryType};
use limine::request::MemoryMapRequest;
use core::mem::drop;
use spin::Mutex; // operations are quite short
use crate::arch::PAGE_SIZE;

#[unsafe(link_section = ".limine_requests")]
static MEMMAP_REQUEST: MemoryMapRequest = MemoryMapRequest::new();

struct FrameLocation {
    region : usize,
    offset : u64,
}

static HEADLOCK: Mutex<u64> = Mutex::new(u64::MAX);
static ENDLOCK: Mutex<FrameLocation> = Mutex::new(FrameLocation{region: 0, offset: 0});

pub fn frame_alloc() -> u64 {
    if let Some(res) = MEMMAP_REQUEST.get_response() {
        // TODO cache get_response
        let mut HEAD = HEADLOCK.lock();
        if *HEAD == u64::MAX {
            drop(HEAD); // not using this anymore
            let mut END = ENDLOCK.lock();
            if END.offset >= res.entries()[END.region].length {
                for region in (END.region+1)..res.entries().len() {
                    if res.entries()[region].entry_type == EntryType::USABLE {
                        *END = FrameLocation{region: region, offset: 0};
                        break;
                    }
                }    
                frame_alloc() // god-awful way to retry in case someone freed a frame
            } else {
                let frame : u64 = res.entries()[END.region].base + END.offset * PAGE_SIZE as u64;
                END.offset += PAGE_SIZE as u64;
                frame
            }
        } else {
            let first : u64 = *HEAD;
            *HEAD = unsafe {*(first as *const u64)};
            first
        }
    } else {
        panic!("limine memory map failure");
    }
}

pub fn frame_dealloc(frame: u64) {
    let mut HEAD = HEADLOCK.lock();
    unsafe {
        *(frame as *mut u64) = *HEAD;
    }
    *HEAD = frame;
    panic!("limine memory map failure"); 
}