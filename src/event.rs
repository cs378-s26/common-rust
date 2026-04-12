use alloc::{boxed::Box, sync::Arc};
use core::sync::atomic::{AtomicUsize, Ordering};

use intrusive_collections::{LinkedList, LinkedListAtomicLink, intrusive_adapter};
use spin::Once;

use crate::{
    arch::{Arch, ArchTrait},
    memory::virtual_memory::{PageFaultConditions, handle_page_fault},
    mp::{CORE_ID, CoreId, core_local},
    sync::{IntSpinLock, MutexLike},
    thread::{CORE_PINNED_TO, LOCAL_WORK_QUEUE, PINNED_TO_CORE, Thread, make_thread, yield_thread},
};

pub enum Event {
    Shootdown {
        space: u64,
        base: usize,
        length: usize,
        latch: Arc<AtomicUsize>,
    },
    PageFault {
        cause: PageFaultConditions,
        address: usize,
        thread: Arc<Thread>,
    },
}
pub struct EventNode {
    event: Event,
    link: LinkedListAtomicLink,
}

intrusive_adapter!(pub EventAdapter = Box<EventNode>: EventNode { link => LinkedListAtomicLink });

core_local! {
    pub EVENT_QUEUE: IntSpinLock<LinkedList<EventAdapter>> = IntSpinLock::new(LinkedList::new(EventAdapter::NEW));
    pub EVENT_HANDLER: Once<Arc<Thread>> = Once::new();
}

pub fn init_event_handler() {
    let thread = make_thread(|| {
        loop {
            while let Some(item) = { EVENT_QUEUE.lock().pop_front() } {
                let EventNode { event, link: _ } = *item;
                use Event::*;
                match event {
                    Shootdown {
                        space: _,
                        base,
                        mut length,
                        latch,
                    } => {
                        // TODO avoid stray invalidations when possible
                        while length > 0 {
                            length -= Arch::PAGE_SIZE;
                            Arch::virtual_invalidate((base + length) as u64);
                        }
                        latch.fetch_sub(1, Ordering::Release);
                    }
                    PageFault {
                        cause,
                        address,
                        thread,
                    } => {
                        handle_page_fault(cause, address, &thread);
                        LOCAL_WORK_QUEUE.lock().push_back(thread);
                    }
                }
            }
            yield_thread(); // TODO block somehow
        }
    });
    PINNED_TO_CORE
        .read_for(&thread)
        .store(true, Ordering::Relaxed);
    CORE_PINNED_TO
        .read_for(&thread)
        .store(CORE_ID.get().0, Ordering::Relaxed);
    EVENT_HANDLER.call_once(|| thread.clone());
    LOCAL_WORK_QUEUE.lock().push_back(thread);
}

pub fn push_event(event: Event, core: CoreId) {
    let queue = EVENT_QUEUE.read_for(core);
    let node = Box::new(EventNode {
        event,
        link: LinkedListAtomicLink::new(),
    });
    queue.lock().push_back(node);
}
