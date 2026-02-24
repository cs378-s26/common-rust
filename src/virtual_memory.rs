use bitflags::bitflags;
use intrusive_collections::{RBTree, RBTreeLink, intrusive_adapter, KeyAdapter, Bound};
use alloc::boxed::Box;
use spin::{Mutex, Once};
use crate::{arch::PAGE_SIZE, physical_memory::HHDM_REQUEST, print::kprintln, vmap, vunmap, get_address_space, frame_alloc};

bitflags! {
    pub struct PageFaultConditions: u64 {
        const PRESENT = 1 << 0;
        const WRITE = 1 << 1;
        const USER  = 1 << 2;
        const CORRUPT = 1 << 3;
        const FETCH = 1 << 4;
    }
}

struct VirtualMemoryEntry {
    base: usize,
    length: usize,
    link: RBTreeLink
}
// https://docs.rs/intrusive-collections/latest/intrusive_collections/
intrusive_adapter!(ActiveTreeAdapter = Box<VirtualMemoryEntry>: VirtualMemoryEntry { link => RBTreeLink });
impl<'a> KeyAdapter<'a> for ActiveTreeAdapter {
    type Key = usize;
    fn get_key(&self, x: &'a VirtualMemoryEntry) -> usize { x.base }
}
intrusive_adapter!(FreeTreeAdapter = Box<VirtualMemoryEntry>: VirtualMemoryEntry { link => RBTreeLink });
impl<'a> KeyAdapter<'a> for FreeTreeAdapter {
    type Key = (usize, usize); // sort by length, but keep distinct keys for each interval
    fn get_key(&self, x: &'a VirtualMemoryEntry) -> (usize, usize) { (x.length, x.base) }
}

struct VirtualMemoryEntryContainer {
    active: RBTree<ActiveTreeAdapter>,
    free: RBTree<FreeTreeAdapter>,
}
// bad bad bad no spinning :( but we can't block yet
static VMES : Once<Mutex<VirtualMemoryEntryContainer>> = Once::new(); // TODO RWLock

pub fn init_virtual_memory_allocator() {
    kprintln!("initializing virtual memory allocator");
    VMES.call_once(|| {
        let mut free = RBTree::new(FreeTreeAdapter::new());
        free.insert(Box::new(VirtualMemoryEntry{base: PAGE_SIZE, length: HHDM_REQUEST.get_response().unwrap().offset() as usize - PAGE_SIZE, link: RBTreeLink::new()}));
        Mutex::new(VirtualMemoryEntryContainer { 
            active: RBTree::new(ActiveTreeAdapter::new()),
            free: free, // one big-ass free block
        })
    });
}

// can't block! (as of now)
pub fn handle_page_fault(cause: PageFaultConditions, address: usize) {
    if !cause.contains(PageFaultConditions::PRESENT) {
        let mut vmes = VMES.get().expect("page fault occurred before virtual memory allocator was initialized").lock();
        if let Some(below) = vmes.active.upper_bound_mut(Bound::Included(&address)).get() {
            assert!(below.base <= address); // can remove once we're confident in this data structure lol
            if below.base + below.length > address {
                let frame = frame_alloc();
                vmap(get_address_space(), address as u64 & (!0xFFF), frame as u64, false, true, true);
            } else {
                panic!("*** PAGE FAULT AT {:x} outside mapped region ***", address);
            }
        } else {
            panic!("*** PAGE FAULT AT {:x} with no VMEs ***", address);
        }
        
    }
}

// brainstormed with ChatGPT for the complementary-tree design, but the code is mine

pub fn virtual_alloc(length: usize) -> usize {
    assert!(length & 0xFFF == 0);
    let mut vmes = VMES.get().expect("virtual allocation attempted before virtual memory allocator was initialized").lock();
    let cursor = &mut vmes.free.lower_bound_mut(Bound::Included(&(length, 0)));
    let mut chosen = cursor.remove().expect("free VME collection error during allocation"); // best-fit allocation
    assert!(chosen.length >= length); // can remove once we're confident in this data structure lol
    vmes.active.insert(Box::new(VirtualMemoryEntry{base: chosen.base, length: length, link: RBTreeLink::new()}));
    let result = chosen.base;
    if chosen.length != length { // don't reinsert duds
        chosen.base += length;
        chosen.length -= length;
        vmes.free.insert(chosen); // need to remove and reinsert because the key changed anyway
    }
    result
}

pub fn virtual_dealloc(base: usize) {
    assert!(base & 0xFFF == 0);
    let mut vmes = VMES.get().expect("Virtual deallocation attempted before virtual memory allocator was initialized").lock();
    let inner = &mut *vmes; // borrow checker lol lmao
    let (active, free) = (&mut inner.active, &mut inner.free);
    let mut cursor = active.find_mut(&base);
    let mut found = cursor.remove().expect("deallocating unallocated virtual address");
    let mut length = found.length;
    // cursor automatically moves to next element
    if let Some(next) = cursor.get() {
        if next.base != found.base + found.length { // remove, automagically drop (free), and merge with entry [found.base + found.length, next.base) in free tree
            let above = free.find(&(next.base - found.base - found.length, found.base + found.length)).get().expect("tree mismatch 1");
            found.length += above.length
        }
    } else if let Some(back) = free.back().get() {
        if back.base > found.base {
            assert!(found.base + found.length == back.base);
            found.length += back.length // merge with topmost free region
        }
    }
    cursor.move_prev();
    if let Some(prev) = cursor.get() {
        if prev.base + prev.length != found.base { // remove, automagically drop (free), and merge with entry [prev.base + prev.length, found.base) in free tree
            let below = free.find(&(found.base - prev.base - prev.length, prev.base + prev.length)).get().expect("tree mismatch 2");
            found.base = below.base;
            found.length += below.length;
        }
    } else if let Some(front) = free.front().get() {
        if front.base < found.base {
            assert!(front.base + front.length == found.base);
            found.base = front.base;
            found.length += front.length;
        }
    }
    // remove any mapped pages from the page table
    while length > 0 {
        vunmap(get_address_space(), (base + length) as u64);
        length -= PAGE_SIZE;
    }
}