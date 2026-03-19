use core::{
    alloc::{GlobalAlloc, Layout},
    cmp::Ordering,
    ptr::{NonNull, null_mut, slice_from_raw_parts_mut},
};

use alloc::boxed::Box;
use spin::Once;
use talc::{ErrOnOom, Span, Talc};

use crate::{
    print::kprintln,
    sync::{IntSpinLock, MutexLike},
};

struct GlobalAllocImpl {
    delegate: Once<IntSpinLock<Talc<ErrOnOom>>>,
}

unsafe impl GlobalAlloc for GlobalAllocImpl {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        let mut delegate = self.delegate.get().expect("alloc not initialized").lock();
        unsafe { delegate.malloc(layout).map_or(null_mut(), |nn| nn.as_ptr()) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
        let mut delegate = self.delegate.get().expect("alloc not initialized").lock();
        unsafe { delegate.free(NonNull::new_unchecked(ptr), layout) };
    }

    unsafe fn realloc(
        &self,
        ptr: *mut u8,
        old_layout: core::alloc::Layout,
        new_size: usize,
    ) -> *mut u8 {
        let delegate = self.delegate.get().expect("alloc not initialized");

        let nn_ptr = unsafe { NonNull::new_unchecked(ptr) };

        match new_size.cmp(&old_layout.size()) {
            Ordering::Greater => {
                if let Ok(nn) =
                    unsafe { delegate.lock().grow_in_place(nn_ptr, old_layout, new_size) }
                {
                    return nn.as_ptr();
                }

                let new_layout =
                    unsafe { Layout::from_size_align_unchecked(new_size, old_layout.align()) };

                let mut lock = delegate.lock();
                let allocation = match unsafe { lock.malloc(new_layout) } {
                    Ok(ptr) => ptr,
                    Err(_) => return null_mut(),
                };

                if old_layout.size() > 0x10000 {
                    drop(lock);
                    unsafe {
                        allocation
                            .as_ptr()
                            .copy_from_nonoverlapping(ptr, old_layout.size())
                    };
                    lock = delegate.lock();
                } else {
                    unsafe {
                        allocation
                            .as_ptr()
                            .copy_from_nonoverlapping(ptr, old_layout.size())
                    };
                }

                unsafe { lock.free(nn_ptr, old_layout) };
                allocation.as_ptr()
            }

            Ordering::Less => {
                unsafe {
                    delegate
                        .lock()
                        .shrink(NonNull::new_unchecked(ptr), old_layout, new_size)
                };
                ptr
            }

            Ordering::Equal => ptr,
        }
    }
}

#[global_allocator]
static GLOBAL_ALLOC: GlobalAllocImpl = GlobalAllocImpl {
    delegate: Once::new(),
};

pub fn init_malloc(memory: Span) {
    kprintln!("mem::init_malloc(): initializing heap");

    GLOBAL_ALLOC.delegate.call_once(|| {
        let mut talc = Talc::new(ErrOnOom);
        unsafe { talc.claim(memory).expect("failed to initialize talc") };
        IntSpinLock::new(talc)
    });
}

pub fn aligned_slice(size: usize, align: usize) -> Box<[u8]> {
    let ptr = unsafe { alloc::alloc::alloc(Layout::from_size_align(size, align).unwrap()) };
    unsafe { Box::from_raw(slice_from_raw_parts_mut(ptr, size)) }
}
