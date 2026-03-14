use core::{
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
};

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::task::Wake;

use intrusive_collections::{LinkedList, LinkedListAtomicLink, intrusive_adapter};
use spin::{Once, RwLock};
// TODO: use a blocking RwLock.

use crate::{
    sync::{IntMutex, MutexLike},
    thread::{spawn_thread, yield_thread},
};

// If we decide we want task IDs again:
// #[repr(transparent)]
// #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
// struct TaskId(u64);

// impl TaskId {
//     fn new() -> Self {
//         // Increment every new instance.
//         static NEXT_ID: AtomicU64 = AtomicU64::new(0);
//         // kprintln!("Creating coroutine with ID {}.", NEXT_ID.load(SeqCst));
//         TaskId(NEXT_ID.fetch_add(1, SeqCst))
//     }
// }

pub struct CoroutineTask {
    future: Pin<Box<dyn Future<Output = ()> + Send + Sync>>, // Send so thread-safe.
}

impl CoroutineTask {
    // Static so future can live.
    pub fn new(future: impl Future<Output = ()> + Send + Sync + 'static) -> CoroutineTask {
        CoroutineTask {
            future: Box::pin(future),
        }
    }

    fn poll(&mut self, context: &mut Context) -> Poll<()> {
        self.future.as_mut().poll(context)
    }
}

// Wrapper for intrusive to add RwLock.
struct CoroutineTaskNode {
    link: LinkedListAtomicLink,
    c: RwLock<CoroutineTask>, // For interior mutability when polling.
}

impl CoroutineTaskNode {
    pub fn new(future: impl Future<Output = ()> + Send + Sync + 'static) -> Self {
        CoroutineTaskNode {
            link: LinkedListAtomicLink::new(),
            c: RwLock::new(CoroutineTask::new(future)),
        }
    }
}

static GLOBAL_EXECUTOR: Once<Executor> = Once::new();

// Intrusive data structures.
intrusive_adapter!(CoroutineQueueAdapter = Arc<CoroutineTaskNode>: CoroutineTaskNode { link => LinkedListAtomicLink });

type CoroutineQueue = LinkedList<CoroutineQueueAdapter>;

fn new_coroutine_queue() -> CoroutineQueue {
    CoroutineQueue::new(CoroutineQueueAdapter::new())
}

struct CoroutineWaker {
    task_node: Arc<CoroutineTaskNode>,
    ready_queue: Arc<IntMutex<CoroutineQueue>>,
}

impl CoroutineWaker {
    fn new_waker(
        task_node: Arc<CoroutineTaskNode>,
        ready_queue: Arc<IntMutex<CoroutineQueue>>,
    ) -> Waker {
        Waker::from(Arc::new(CoroutineWaker {
            task_node,
            ready_queue,
        }))
    }

    fn wake_task(&self) {
        self.ready_queue.lock().push_back(self.task_node.clone());
    }
}

impl Wake for CoroutineWaker {
    fn wake(self: Arc<Self>) {
        self.wake_task();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.wake_task();
    }
}

struct Executor {
    ready_queue: Arc<IntMutex<CoroutineQueue>>,
}

impl Executor {
    fn new() -> Self {
        Executor {
            ready_queue: Arc::new(IntMutex::new(new_coroutine_queue())),
        }
    }

    pub fn run(&self) {
        const BATCH_SIZE: usize = 20;
        loop {
            for _ in 0..BATCH_SIZE {
                let task_node = { self.ready_queue.lock().pop_front() };

                let Some(task_node) = task_node else {
                    break; // Empty queue.
                };

                // Create new waker.
                let waker = CoroutineWaker::new_waker(task_node.clone(), self.ready_queue.clone());
                let mut context = Context::from_waker(&waker);

                // Make progress.
                let _ = task_node.c.write().poll(&mut context);
                // If not ready, waker will put back in queue.
            }
            yield_thread();
        }
    }

    fn spawn(&self, future: impl Future<Output = ()> + Send + Sync + 'static) {
        let task = CoroutineTaskNode::new(future);
        self.ready_queue.lock().push_back(Arc::new(task));
    }
}

pub fn spawn_coroutine(future: impl Future<Output = ()> + Send + Sync + 'static) {
    GLOBAL_EXECUTOR.get().unwrap().spawn(future);
}

pub fn init_coroutine_queue() {
    GLOBAL_EXECUTOR.call_once(Executor::new);
}

pub fn init_coroutine_executor() {
    spawn_thread(|| GLOBAL_EXECUTOR.get().unwrap().run());
}
