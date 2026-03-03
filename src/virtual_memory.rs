use crate::{
    arch::{Arch, ArchTrait},
    physical_memory::{HHDM_REQUEST, frame_alloc},
    print::kprintln,
};
use alloc::boxed::Box;
use bitflags::bitflags;
use intrusive_collections::{Bound, KeyAdapter, RBTree, RBTreeLink, intrusive_adapter};
use spin::{Mutex, Once};
use core::ops::{Deref, DerefMut};

bitflags! {
    pub struct PageFaultConditions: u64 {
        const PRESENT = 1 << 0;
        const WRITE = 1 << 1;
        const USER  = 1 << 2;
        const CORRUPT = 1 << 3;
        const FETCH = 1 << 4;
    }
}

bitflags! {
    #[derive(Copy, Clone)]
    pub struct PagingOptions: u64 {
        const PRESENT = 1 << 0;
        const WRITABLE = 1 << 1;
        const EXECUTABLE = 1 << 2;
        const USER_ACCESSIBLE = 1 << 3;
        const WRITE_THROUGH = 1 << 4;
        const CACHEABLE = 1 << 5;
        const GLOBAL = 1 << 6;
    }
}

struct VirtualMemoryEntry {
    base: usize,
    length: usize,
    // future proofing
    #[allow(unused)]
    options: PagingOptions,
    link: RBTreeLink,
}
// https://docs.rs/intrusive-collections/latest/intrusive_collections/
intrusive_adapter!(ActiveTreeAdapter = Box<VirtualMemoryEntry>: VirtualMemoryEntry { link => RBTreeLink });
impl<'a> KeyAdapter<'a> for ActiveTreeAdapter {
    type Key = usize;
    fn get_key(&self, x: &'a VirtualMemoryEntry) -> usize {
        x.base
    }
}
intrusive_adapter!(FreeTreeAdapter = Box<VirtualMemoryEntry>: VirtualMemoryEntry { link => RBTreeLink });
impl<'a> KeyAdapter<'a> for FreeTreeAdapter {
    type Key = (usize, usize); // sort by length, but keep distinct keys for each interval
    fn get_key(&self, x: &'a VirtualMemoryEntry) -> (usize, usize) {
        (x.length, x.base)
    }
}

struct VirtualMemoryEntryContainer {
    active: RBTree<ActiveTreeAdapter>,
    free: RBTree<FreeTreeAdapter>,
}
// bad bad bad no spinning :( but we can't block yet
static VMES: Once<Mutex<VirtualMemoryEntryContainer>> = Once::new(); // TODO RWLock

pub fn init_virtual_memory_allocator() {
    kprintln!("initializing virtual memory allocator");
    VMES.call_once(|| {
        let mut free = RBTree::new(FreeTreeAdapter::new());
        free.insert(Box::new(VirtualMemoryEntry {
            base: Arch::PAGE_SIZE,
            length: HHDM_REQUEST.get_response().unwrap().offset() as usize - Arch::PAGE_SIZE,
            options: PagingOptions::empty(),
            link: RBTreeLink::new(),
        }));
        Mutex::new(VirtualMemoryEntryContainer {
            active: RBTree::new(ActiveTreeAdapter::new()),
            free, // one big-ass free block
        })
    });
}

// can't block! (as of now)
pub fn handle_page_fault(space: u64, address: usize, cause: PageFaultConditions) {
    if !cause.contains(PageFaultConditions::PRESENT) {
        let mut vmes = VMES
            .get()
            .expect("page fault occurred before virtual memory allocator was initialized")
            .lock();
        if let Some(below) = vmes.active.upper_bound_mut(Bound::Included(&address)).get() {
            assert!(below.base <= address); // can remove once we're confident in this data structure lol
            if below.base + below.length > address {
                let frame = frame_alloc();
                Arch::virtual_map(
                    Arch::get_address_space(),
                    address as u64 & (!0xFFF),
                    frame as u64,
                    PagingOptions::PRESENT | PagingOptions::WRITABLE | PagingOptions::CACHEABLE, // TODO use vme options
                );
            } else {
                panic!("*** PAGE FAULT AT {:x} outside mapped region ***", address);
            }
        } else {
            panic!("*** PAGE FAULT AT {:x} with no VMEs ***", address);
        }
    }
}

#[derive(Clone, Copy)]
pub struct VirtualMemoryRange {
    pub space: u64,
    pub base: usize,
    pub length: usize,
}

pub struct VirtualMemoryAllocation(VirtualMemoryRange);

impl Deref for VirtualMemoryAllocation {
    type Target = VirtualMemoryRange;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for VirtualMemoryAllocation {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

// brainstormed with ChatGPT for the complementary-tree design, but the code is mine
impl VirtualMemoryAllocation {
    pub fn new(
        space: u64,
        length: usize,
        backing: Option<usize>,
        options: PagingOptions,
    ) -> VirtualMemoryAllocation {
        assert!(length & 0xFFF == 0);
        let mut vmes = VMES
            .get()
            .expect("virtual allocation attempted before virtual memory allocator was initialized")
            .lock();
        let cursor = &mut vmes.free.lower_bound_mut(Bound::Included(&(length, 0)));
        let mut chosen = cursor
            .remove()
            .expect("free VME collection error during allocation"); // best-fit allocation
        assert!(chosen.length >= length); // can remove once we're confident in this data structure lol
        vmes.active.insert(Box::new(VirtualMemoryEntry {
            base: chosen.base,
            length,
            options,
            link: RBTreeLink::new(),
        }));
        let base = chosen.base;
        if chosen.length != length {
            // don't reinsert duds
            chosen.base += length;
            chosen.length -= length;
            vmes.free.insert(chosen); // need to remove and reinsert because the key changed anyway
        }
        if let Some(physical) = backing {
            let mut i = 0;
            while i < length {
                Arch::virtual_map(space, (base + i) as u64, (physical + i) as u64, options);
                i += Arch::PAGE_SIZE;
            }
        }
        VirtualMemoryAllocation(VirtualMemoryRange {
            space,
            base,
            length,
        })
    }
}

impl Drop for VirtualMemoryAllocation {
    fn drop(&mut self) {
        // remove any mapped pages from the page table
        while self.length > 0 {
            self.length -= Arch::PAGE_SIZE;
            Arch::virtual_unmap(self.space, (self.base + self.length) as u64);
        }
        let mut vmes = VMES
            .get()
            .expect(
                "Virtual deallocation attempted before virtual memory allocator was initialized",
            )
            .lock();
        let inner = &mut *vmes; // borrow checker lol lmao
        let (active, free) = (&mut inner.active, &mut inner.free);
        let mut cursor = active.find_mut(&self.base);
        let mut found = cursor
            .remove()
            .expect("deallocating unallocated virtual address");
        // cursor automatically moves to next element
        if let Some(next) = cursor.get() {
            if next.base != found.base + found.length {
                // remove, automagically drop (free), and merge with entry [found.base + found.length, next.base) in free tree
                let above = free
                    .find(&(
                        next.base - found.base - found.length,
                        found.base + found.length,
                    ))
                    .get()
                    .expect("tree mismatch 1");
                found.length += above.length
            }
        } else if let Some(back) = free.back().get()
            && back.base > found.base
        {
            assert!(found.base + found.length == back.base);
            found.length += back.length // merge with topmost free region
        }
        cursor.move_prev();
        if let Some(prev) = cursor.get() {
            if prev.base + prev.length != found.base {
                // remove, automagically drop (free), and merge with entry [prev.base + prev.length, found.base) in free tree
                let below = free
                    .find(&(
                        found.base - prev.base - prev.length,
                        prev.base + prev.length,
                    ))
                    .get()
                    .expect("tree mismatch 2");
                found.base = below.base;
                found.length += below.length;
            }
        } else if let Some(front) = free.front().get()
            && front.base < found.base
        {
            assert!(front.base + front.length == found.base);
            found.base = front.base;
            found.length += front.length;
        }
    }
}

#[cfg(test)]
mod test {
    use super::kprintln;
    use crate::arch::{Arch, ArchTrait};
    use crate::physical_memory::{frame_alloc, frame_dealloc};
    use crate::thread::{spawn_thread, yield_thread};
    use crate::virtual_memory::{PagingOptions, VirtualMemoryAllocation};
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicI64, Ordering};

    #[test_case]
    fn test_virtual_memory() {
        let barrier = Arc::new(AtomicI64::new(2));
        for tid in 1..3 {
            let b = barrier.clone();
            kprintln!("creating thread");
            spawn_thread(move || {
                kprintln!("starting thread");
                let vaddr: u64 = 0x10000000 * tid; // unsafe!
                let frame_1: usize = frame_alloc();
                kprintln!("frame 1: {:x}", frame_1);
                let frame_2: usize = frame_alloc();
                kprintln!("frame 2: {:x}", frame_2);
                frame_dealloc(frame_1);

                kprintln!("manually mapping vmem");
                Arch::virtual_map(
                    Arch::get_address_space(),
                    vaddr,
                    frame_2 as u64,
                    PagingOptions::PRESENT
                        | PagingOptions::WRITABLE
                        | PagingOptions::CACHEABLE
                        | PagingOptions::GLOBAL,
                );
                kprintln!("writing to manually mapped vmem");
                for i in 0..4096 {
                    unsafe { *((vaddr + i) as *mut u8) = i as u8 };
                }
                kprintln!("reading from manually mapped vmem");
                for i in 0..4096 {
                    assert!(unsafe { *((vaddr + i) as *mut u8) } == i as u8);
                }
                kprintln!("manually unmapping vmem");
                Arch::virtual_unmap(Arch::get_address_space(), vaddr);

                kprintln!("properly mapping vmem");
                let mmapped = VirtualMemoryAllocation::new(
                    Arch::get_address_space(),
                    0x3000,
                    None,
                    PagingOptions::PRESENT
                        | PagingOptions::WRITABLE
                        | PagingOptions::CACHEABLE
                        | PagingOptions::GLOBAL,
                );
                kprintln!("writing to properly mapped vmem");
                for i in 0..4096 {
                    unsafe { *((mmapped.base + i) as *mut u8) = i as u8 };
                }
                for i in 0..4096 {
                    unsafe { *((mmapped.base + i) as *mut u8) = i as u8 };
                }
                kprintln!("reading from properly mapped vmem");
                for i in 8192..8192 + 4096 {
                    unsafe { *((mmapped.base + i) as *mut u8) = i as u8 };
                }
                for i in 8192..8192 + 4096 {
                    unsafe { *((mmapped.base + i) as *mut u8) = i as u8 };
                }
                kprintln!("properly unmapping vmem");
                drop(mmapped);
                (*b).fetch_add(-1, Ordering::SeqCst);
                loop {
                    yield_thread();
                }
            });
        }
        kprintln!("waiting at barrier");
        while (*barrier).load(Ordering::SeqCst) > 0 {
            yield_thread();
        }
    }
}
