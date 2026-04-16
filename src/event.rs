use alloc::{boxed::Box, sync::Arc};
use core::sync::atomic::{AtomicUsize, Ordering};

use intrusive_collections::{LinkedList, LinkedListAtomicLink, intrusive_adapter};
use spin::Once;
use crate::syscall::{SyscallContext, syscall_handler};

use crate::{
    arch::{Arch, ArchTrait},
    memory::virtual_memory::{PageFaultConditions, handle_page_fault},
    mp::{CORE_ID, CoreId, core_local},
    sync::{IntSpinLock, MutexLike},
    thread::{CORE_PINNED_TO, CUR_EVENT, LOCAL_WORK_QUEUE, PINNED_TO_CORE, Thread, make_thread, yield_thread},
    virtual_memory::{PageFaultConditions, handle_page_fault},
    thread::{ThreadQueue, ThreadQueueAdapter, this_thread}
};

pub enum Event<S: SyscallContext> {
    Shootdown {
        space: u64,
        base: usize,
        length: usize,
        latch: Arc<AtomicUsize>,
    },
    PageFault {
        cause: PageFaultConditions,
        address: usize,
        //thread: Arc<Thread>,
    },
    Syscall {
        context: S,
    }
}

pub struct EventNode {
    //I would use an enum here but then mem alloc :(
    event: Event,
    link: LinkedListAtomicLink,
}

intrusive_adapter!(pub EventAdapter = Box<EventNode>: EventNode { link => LinkedListAtomicLink });

core_local! {
    pub EVENT_QUEUE: IntSpinLock<LinkedList<EventAdapter>> = IntSpinLock::new(LinkedList::new(EventAdapter::NEW));
    pub EVENT_THREAD_QUEUE: IntSpinLock<ThreadQueue> = IntSpinLock::new(ThreadQueue::new(ThreadQueueAdapter::NEW));
    pub EVENT_HANDLER: Once<Arc<Thread>> = Once::new();
}

pub fn init_event_handler() {
    let thread = make_thread(|| {
        loop {
            if let Some(item) = { EVENT_QUEUE.lock().pop_front() } {
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
                    PageFault {..} => {
                        panic!("Page fault events should never be pushed to the event queue, they should always be handled immediately by the thread that caused the page fault");
                    }
                }
            }
            if let Some(thread) = { EVENT_THREAD_QUEUE.lock().pop_front() } {
                // this is O(1) even though the compiler doesn't know it
                // since there should never be any contention here
                // also queue insertions do not require any memory allocations since
                // we are using intrusive linked lists
                let event = CUR_EVENT.read_for(&thread).lock().take().unwrap();
                match event {
                    Event::PageFault { cause, address } => {
                        handle_page_fault(cause, address);
                        LOCAL_WORK_QUEUE.lock().push_back(thread);
                    }
                    Event::Shootdown { .. } => {
                        panic!("Shootdown events should never be pushed to the thread event queue, they should always be handled immediately by the thread that caused the shootdown");
                    }
                    _ => unreachable!(),
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

pub fn push_event(event: Event, core: CoreId, should_alloc : bool) {
    let queue = EVENT_QUEUE.read_for(core);
    if should_alloc {
        // if the event is being pushed from an interrupt handler, we need to allocate the event node on the heap
        // because the interrupt handler may be preempted by another thread that also tries to push an event, which would cause a double free if we used a stack allocated node
        let node: Box<EventNode> = Box::new(EventNode {
            event,
            link: LinkedListAtomicLink::new(),
        });
        queue.lock().push_back(node);
    } else {
        // if the event is being pushed from a thread, we can allocate the event node on the stack
        CUR_EVENT.lock().replace(event);
        //this is O(1) even though the compiler doesn't know it
        //since there should never be any contention here
        //also queue insertions do not require any memory allocations since
        //we are using intrusive linked lists
        EVENT_THREAD_QUEUE.lock().push_back(this_thread());
    }
}
