use crate::{
    arch::{Arch, ArchTrait},
    mp::{CoreId, core_local},
    thread::{Thread, make_thread, yield_thread},
    virtual_memory::{PageFaultConditions, VirtualMemoryRange, handle_page_fault},
};
use alloc::{boxed::Box, sync::Arc};
use core::sync::atomic::{AtomicUsize, Ordering};
use intrusive_collections::{LinkedList, LinkedListAtomicLink, intrusive_adapter};
use spin::{Mutex, Once};

pub enum Event {
    Shootdown {
        range: VirtualMemoryRange,
        latch: Arc<AtomicUsize>,
    },
    PageFault {
        space: u64,
        address: usize,
        cause: PageFaultConditions,
    },
}
pub struct EventNode {
    event: Event,
    link: LinkedListAtomicLink,
}

intrusive_adapter!(pub EventAdapter = Box<EventNode>: EventNode { link => LinkedListAtomicLink });

core_local! {
    pub EVENT_HANDLER: Once<Arc<Thread>> = Once::new();
    pub EVENT_QUEUE: Once<Mutex<LinkedList<EventAdapter>>> = Once::new();
}

pub fn init_event_handler() {
    EVENT_HANDLER.call_once(|| {
        make_thread(
            || {
                loop {
                    while let Some(item) = EVENT_QUEUE.get().unwrap().lock().pop_front() {
                        let EventNode { event, link: _ } = *item;
                        use Event::*;
                        match event {
                            Shootdown { mut range, latch } => {
                                while range.length > 0 {
                                    range.length -= Arch::PAGE_SIZE;
                                    Arch::tlb_flush((range.base + range.length) as u64);
                                }
                                latch.fetch_add(1, Ordering::Relaxed);
                            }
                            PageFault {
                                space,
                                address,
                                cause,
                            } => {
                                handle_page_fault(space, address, cause);
                            }
                        }
                    }
                    yield_thread();
                }
            },
            true,
        )
    }); // TODO make this preemptible
    EVENT_QUEUE.call_once(|| Mutex::new(LinkedList::new(EventAdapter::new())));
}

pub fn push_event(event: Event, core: CoreId) {
    let queue = EVENT_QUEUE.read_for(core);
    queue.get().unwrap().lock().push_back(Box::new(EventNode {
        event,
        link: LinkedListAtomicLink::new(),
    }));
}

pub fn is_pending_event() -> bool {
    !EVENT_QUEUE.get().unwrap().lock().front().is_null()
}
