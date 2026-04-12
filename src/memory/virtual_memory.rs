use alloc::{boxed::Box, sync::Arc};

use bitflags::bitflags;
use intrusive_collections::{Bound, KeyAdapter, RBTree, RBTreeLink, intrusive_adapter};
use limine::{memory_map::EntryType, request::ExecutableAddressRequest};
use spin::{Mutex, Once};

use crate::{
    arch::{Arch, ArchTrait},
    memory::{
        physical_memory::{HHDM_OFFSET, REGIONS, frame_alloc},
        virtual_memory_2::USERSPACE_END,
    },
    print::kprintln,
    state::{CorePin, StateGuard},
    thread::Thread,
};

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
        const FIXED = 1 << 7;
        const SHADOW = 1 << 8;
        const DEVICE_MEMORY = 1 << 9;
    }
}

struct VirtualMemoryEntry {
    pub base: usize,   // lowest address in range
    pub length: usize, // size of range
    #[allow(unused)]
    pub options: PagingOptions, // architecture-independent
    link: RBTreeLink,  // for the intrustive trees
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

// bad bad bad no spinning :( but we can't block yet
static VMES: Once<Mutex<VirtualMemoryEntryContainer>> = Once::new(); // TODO RWLock

#[unsafe(link_section = ".limine_requests")]
static EXECUTABLE_ADDRESS_REQUEST: ExecutableAddressRequest = ExecutableAddressRequest::new();

struct VirtualMemoryEntryContainer {
    low: usize,
    high: usize,
    active: RBTree<ActiveTreeAdapter>,
    free: RBTree<FreeTreeAdapter>,
}

impl VirtualMemoryEntryContainer {
    pub fn new(low: usize, high: usize) -> VirtualMemoryEntryContainer {
        let mut free = RBTree::new(FreeTreeAdapter::new());
        free.insert(Box::new(VirtualMemoryEntry {
            base: low,
            length: high - low,
            options: PagingOptions::empty(),
            link: RBTreeLink::new(),
        }));
        VirtualMemoryEntryContainer {
            low,
            high,
            active: RBTree::new(ActiveTreeAdapter::new()),
            free,
        }
    }
}

pub fn init_virtual_memory_allocator() {
    Arch::configure_vm();
    kprintln!("initializing virtual memory allocator");
    VMES.call_once(|| {
        Mutex::new(VirtualMemoryEntryContainer::new(
            *HHDM_OFFSET.get().unwrap(),
            usize::MAX & !(Arch::PAGE_SIZE - 1),
        ))
    });
    let mut executable_length = 0;
    let executable_start = EXECUTABLE_ADDRESS_REQUEST.get_response().unwrap();
    for region in *REGIONS.get().unwrap() {
        // if you need to map over one of these, just change backing and options accordingly
        VirtualMemoryAllocation::new(
            Arch::get_kernel_address_space(),
            Some(HHDM_OFFSET.get().unwrap() + region.base as usize),
            region.length as usize,
            None,
            PagingOptions::SHADOW,
            true,
        );
        if region.entry_type == EntryType::EXECUTABLE_AND_MODULES
            && (region.base) <= executable_start.physical_base()
            && executable_start.physical_base() < (region.base + region.length)
        {
            assert!(
                executable_length == 0,
                "multiple executable sections, kernel mapping unknown"
            );
            executable_length = region.length;
        }
    }
    assert!(
        executable_length > 0,
        "kernel executable section not found, kernel mapping unknown"
    );
    kprintln!(
        "kernel virtually mapped from {:x} to {:x}",
        executable_start.virtual_base(),
        executable_start.virtual_base() + executable_length
    );
    VirtualMemoryAllocation::new(
        Arch::get_kernel_address_space(),
        Some(executable_start.virtual_base() as usize),
        executable_length as usize,
        None,
        PagingOptions::SHADOW,
        true,
    );
}

pub fn handle_page_fault(cause: PageFaultConditions, address: usize, thread: &Arc<Thread>) {
    if address < USERSPACE_END {
        if let Some(process) = thread.process.get() {
            process
                .virtual_memory
                .handle_page_fault(cause, address)
                .unwrap();
        } else {
            panic!("*** PAGE FAULT AT {:x} when no process exists ***", address);
        }
        return;
    }
    if !cause.contains(PageFaultConditions::PRESENT) {
        let mut vmes = VMES
            .get()
            .expect("page fault occurred before virtual memory allocator was initialized")
            .lock();
        if let Some(below) = vmes.active.upper_bound_mut(Bound::Included(&address)).get()
            && below.base + below.length > address
        {
            drop(vmes);
            let frame = frame_alloc();
            Arch::virtual_map(
                Arch::get_kernel_address_space(),
                address as u64 & !(Arch::PAGE_SIZE as u64 - 1),
                frame as u64,
                PagingOptions::PRESENT | PagingOptions::WRITABLE | PagingOptions::CACHEABLE,
            );
        } else {
            for vme in vmes.active.iter() {
                kprintln!(
                    "mapping from {:x} to {:x}: {}",
                    vme.base,
                    vme.base + vme.length,
                    if vme.options.contains(PagingOptions::SHADOW) {
                        "shadow"
                    } else {
                        "real"
                    }
                ) // TODO expand dump
            }
            panic!("*** PAGE FAULT AT {:x} outside mapped region ***", address);
        }
    }
}

pub struct VirtualMemoryAllocation {
    pub space: u64,
    pub base: usize,
    pub length: usize,
    /// When false, Drop unmaps pages but does not free the physical frames.
    /// Used for MMIO where physical addresses belong to hardware, not RAM.
    pub owns_backing: bool,
}

// brainstormed with ChatGPT for the complementary-tree design, but the code is mine
impl VirtualMemoryAllocation {
    pub fn new(
        space: u64,             // address space identifier
        start: Option<usize>,   // fixed mapping location
        length: usize,          // requested size in bytes
        backing: Option<usize>, // physical frames used
        options: PagingOptions, // similar to mmap flags
        owns_backing: bool,     // if false, Drop won't free physical frames
    ) -> Option<VirtualMemoryAllocation> {
        assert!(length.is_multiple_of(Arch::PAGE_SIZE));
        let mut vmes = VMES
            .get()
            .expect("virtual allocation attempted before virtual memory allocator was initialized")
            .lock();

        let mut chosen = if let Some(base) = start {
            // TODO there ought to be a better way to find the free region that contains this fixed region
            let mut cursor = vmes.active.upper_bound(Bound::Excluded(&base));
            let bottom = if let Some(entry) = cursor.get() {
                entry.base + entry.length
            } else {
                vmes.low
            };
            cursor.move_next();
            let top = if let Some(entry) = cursor.get() {
                entry.base
            } else {
                vmes.high
            };
            let mut chosen = vmes
                .free
                .find_mut(&(top - bottom, bottom))
                .remove()
                .expect("fixed mapping attempted at unavailable base");
            if chosen.base < base {
                // cut off bottom part
                vmes.free.insert(Box::new(VirtualMemoryEntry {
                    base: chosen.base,
                    length: base - chosen.base,
                    options: PagingOptions::empty(),
                    link: RBTreeLink::new(),
                }));
                chosen.length -= base - chosen.base;
                chosen.base = base;
            }
            chosen
        } else {
            // find best (smallest) fit
            vmes.free
                .lower_bound_mut(Bound::Included(&(length, 0)))
                .remove()
                .expect("free VME collection error during allocation") // best-fit allocation
        };
        assert!(chosen.length >= length); // can remove once we're confident in this data structure lol

        // reinsert remaining piece (if any) of free block
        let base = chosen.base;
        if chosen.length != length {
            chosen.base += length;
            chosen.length -= length;
            vmes.free.insert(chosen); // need to remove and reinsert because the key changed anyway
        }

        vmes.active.insert(Box::new(VirtualMemoryEntry {
            base,
            length,
            options,
            link: RBTreeLink::new(),
        }));

        drop(vmes); // not using these anymore

        if let Some(physical) = backing {
            let mut i = 0;
            while i < length {
                Arch::virtual_map(space, (base + i) as u64, (physical + i) as u64, options);
                i += Arch::PAGE_SIZE;
            }
        }

        if options.contains(PagingOptions::SHADOW) {
            None
        } else {
            Some(VirtualMemoryAllocation {
                space,
                base,
                owns_backing,
                length,
            })
        }
    }
}

impl Drop for VirtualMemoryAllocation {
    fn drop(&mut self) {
        // remove any mapped pages from the page table
        let mut length = self.length;
        let guard = StateGuard::<CorePin>::guard();
        while length > 0 {
            length -= Arch::PAGE_SIZE;
            if self.owns_backing {
                Arch::virtual_unmap(self.space, (self.base + length) as u64);
            } else {
                Arch::virtual_unmap_no_dealloc(self.space, (self.base + length) as u64);
            }
        }

        // invalidate all cores' TLBs
        Arch::shootdown_tlbs(self.space, self.base, self.length);
        drop(guard);

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
                    .find_mut(&(
                        next.base - found.base - found.length,
                        found.base + found.length,
                    ))
                    .remove()
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
            assert!(prev.base + prev.length <= found.base);
            if prev.base + prev.length != found.base {
                // remove, automagically drop (free), and merge with entry [prev.base + prev.length, found.base) in free tree
                let below = free
                    .find_mut(&(
                        found.base - prev.base - prev.length,
                        prev.base + prev.length,
                    ))
                    .remove()
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
        free.insert(found);
    }
}

#[cfg(test)]
mod test {

    use alloc::{sync::Arc, vec::Vec};

    use spin::{Mutex, barrier::Barrier};

    use super::kprintln;
    use crate::{
        arch::{Arch, ArchTrait},
        memory::{
            physical_memory::{HHDM_OFFSET, frame_alloc, frame_dealloc},
            virtual_memory::{PagingOptions, VirtualMemoryAllocation},
        },
        thread::spawn_thread,
    };

    #[test_case]
    fn test_manual_page_mapping() {
        kprintln!("virtual memory mapping test started");
        frame_dealloc(frame_alloc()); // quick check that frame allocator works
        kprintln!("frame allocator sanity-checked");
        let hhdm = HHDM_OFFSET.get().unwrap();

        // bit of a clunky edit to the test, but for aarch64 currently only higher half mappings are available
        // because of the different page tables for higher and lower half, but on x86 we can't map something that's
        // already mapped, so we just unmap first and make sure remapping works
        let vaddr: usize = 0x1000 + hhdm; // unsafe!
        Arch::virtual_unmap_no_dealloc(Arch::get_kernel_address_space(), vaddr as u64);
        let paddr: usize = frame_alloc();

        kprintln!("manually mapping vmem");
        Arch::virtual_map(
            Arch::get_kernel_address_space(),
            vaddr as u64,
            paddr as u64,
            PagingOptions::PRESENT
                | PagingOptions::WRITABLE
                | PagingOptions::CACHEABLE
                | PagingOptions::GLOBAL,
        );
        kprintln!("writing to manually mapped vmem");
        for i in 0..Arch::PAGE_SIZE {
            unsafe { *((vaddr + i) as *mut u8) = i as u8 };
        }
        kprintln!("reading from manually mapped vmem");
        for i in 0..Arch::PAGE_SIZE {
            assert!(unsafe { *((vaddr + i) as *mut u8) } == i as u8);
        }
        kprintln!("manually unmapping vmem");
        Arch::virtual_unmap(Arch::get_kernel_address_space(), vaddr as u64);
        kprintln!("virtual memory mapping test complete");
    }

    #[test_case]
    fn test_virtual_memory_allocation() {
        kprintln!("virtual memory allocation test started");
        const SIZE: usize = 3 * Arch::PAGE_SIZE;
        kprintln!("properly mapping vmem");
        let mmapped = (
            VirtualMemoryAllocation::new(
                Arch::get_kernel_address_space(),
                None,
                SIZE,
                None,
                PagingOptions::PRESENT
                    | PagingOptions::WRITABLE
                    | PagingOptions::CACHEABLE
                    | PagingOptions::GLOBAL,
                true,
            )
            .unwrap(),
            VirtualMemoryAllocation::new(
                Arch::get_kernel_address_space(),
                None,
                SIZE,
                None,
                PagingOptions::PRESENT
                    | PagingOptions::WRITABLE
                    | PagingOptions::CACHEABLE
                    | PagingOptions::GLOBAL,
                true,
            )
            .unwrap(),
        );
        kprintln!("writing to properly mapped vmem");
        for i in 0..SIZE {
            unsafe { *((mmapped.0.base + i) as *mut u8) = i as u8 };
            unsafe { *((mmapped.1.base + i) as *mut u8) = i as u8 };
        }
        kprintln!("reading from properly mapped vmem");
        for i in 0..SIZE {
            assert!(unsafe { *((mmapped.0.base + i) as *mut u8) } == i as u8);
            assert!(unsafe { *((mmapped.1.base + i) as *mut u8) } == i as u8);
        }
        kprintln!("properly unmapping vmem");
        drop(mmapped);
        kprintln!("virtual memory allocation test complete");
    }

    // TODO potential tests: options, ...

    fn rand(seed: &mut u8) -> u8 {
        // TODO use real OS pseudorandomness
        *seed = (*seed).wrapping_mul(37);
        *seed = (*seed).rotate_right(3);
        *seed ^= 0xA5;
        *seed
    }

    #[test_case]
    fn test_virtual_memory_patterns() {
        kprintln!("virtual memory patterns test started");
        const ITERATIONS: usize = 64;
        let mut seed = 0xed;
        let mut vmas = Vec::new();
        for _ in 0..ITERATIONS {
            if vmas.is_empty() || rand(&mut seed).is_multiple_of(2) {
                let vma = VirtualMemoryAllocation::new(
                    Arch::get_kernel_address_space(),
                    None,
                    Arch::PAGE_SIZE * rand(&mut seed) as usize,
                    None,
                    PagingOptions::PRESENT
                        | PagingOptions::WRITABLE
                        | PagingOptions::CACHEABLE
                        | PagingOptions::GLOBAL,
                    true,
                )
                .unwrap();
                for j in (0..vma.length).step_by(Arch::PAGE_SIZE) {
                    // write to every page in the allocation
                    unsafe { *((vma.base + j) as *mut u8) = j as u8 | 0x80 };
                }
                vmas.push(vma);
            } else {
                let vma = vmas.remove((rand(&mut seed) as usize) % vmas.len());
                for j in (0..vma.length).step_by(Arch::PAGE_SIZE) {
                    // check every page in the allocation
                    assert!(unsafe { *((vma.base + j) as *mut u8) } == j as u8 | 0x80);
                }
            }
        }
        while let Some(vma) = vmas.pop() {
            for j in (0..vma.length).step_by(Arch::PAGE_SIZE) {
                // check every page in the allocation
                assert!(unsafe { *((vma.base + j) as *mut u8) } == j as u8 | 0x80);
            }
        }
        kprintln!("virtual memory patterns test completed");
    }

    #[test_case]
    fn test_virtual_memory_threading() {
        // TODO! why is this only dealloc'ing one VA?
        kprintln!("virtual memory threading test started");
        const THREADS: usize = 8;
        const ITERATIONS: usize = 16;
        let thread_barrier: Arc<Barrier> = Arc::new(Barrier::new(THREADS));
        let test_barrier: Arc<Barrier> = Arc::new(Barrier::new(THREADS + 1));
        let bases: Arc<Mutex<Vec<VirtualMemoryAllocation>>> = Arc::new(Mutex::new(Vec::new()));
        for _ in 0..THREADS {
            let thread_barrier = thread_barrier.clone();
            let test_barrier = test_barrier.clone();
            let thread_bases = bases.clone();
            spawn_thread(move || {
                for i in 0..ITERATIONS {
                    let size = Arch::PAGE_SIZE * (i + 1);
                    let mmapped = VirtualMemoryAllocation::new(
                        Arch::get_kernel_address_space(),
                        None,
                        size,
                        None,
                        PagingOptions::PRESENT
                            | PagingOptions::WRITABLE
                            | PagingOptions::CACHEABLE
                            | PagingOptions::GLOBAL,
                        true,
                    )
                    .unwrap(); // allocations of increasing sizes
                    for j in (0..size).step_by(Arch::PAGE_SIZE) {
                        // write to every page in the allocation
                        unsafe { *((mmapped.base + j) as *mut u8) = j as u8 };
                    }
                    let mut lock = thread_bases.lock();
                    (*lock).push(mmapped);
                    drop(lock);
                    (*thread_barrier).wait();
                    for t in 0..THREADS {
                        let lock = thread_bases.lock();
                        let vma = lock[t].base;
                        drop(lock);
                        for j in (0..size).step_by(Arch::PAGE_SIZE) {
                            // read from every page in every allocation
                            assert!(unsafe { *((vma + j) as *mut u8) } == j as u8);
                        }
                    }
                    (*thread_barrier).wait();
                    let mut lock = thread_bases.lock();
                    lock.pop();
                    drop(lock);
                    (*thread_barrier).wait();
                }
                (*test_barrier).wait();
            });
        }
        (*test_barrier).wait();
        kprintln!("virtual memory threading test complete");
    }
}
