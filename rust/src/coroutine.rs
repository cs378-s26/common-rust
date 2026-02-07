use core::{
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
    sync::atomic::{AtomicU64, Ordering::SeqCst},
    cmp::Ordering,
};

use alloc::task::Wake;
use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::sync::Arc;

use intrusive_collections::{RBTreeAtomicLink, RBTree, LinkedList, LinkedListAtomicLink,
    intrusive_adapter, KeyAdapter};
use spin::{Once, RwLock};
// TODO: use a blocking RwLock.

use crate::{
    thread::{spawn_thread, yield_thread},
    print::kprintln,
    sync::IntMutex,
};

#[repr(transparent)]
struct TaskId {
    id: u64,
}

impl TaskId {
    fn new() -> Self {
        // Increment every new instance.
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        // kprintln!("Creating coroutine with ID {}.", NEXT_ID.load(SeqCst));
        TaskId { id: NEXT_ID.fetch_add(1, SeqCst) }
    }
}

impl Clone for TaskId {
    fn clone(&self) -> Self {
        TaskId { id: self.id }
    }
}

impl Copy for TaskId {}

// Ordering for TaskId.
impl PartialEq for TaskId {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for TaskId {}

impl PartialOrd for TaskId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TaskId {
    fn cmp(&self, other: &Self) -> Ordering {
        self.id.cmp(&other.id)
    }
}

pub struct CoroutineTask {
    future: Pin<Box<dyn Future<Output = ()> + Send + Sync>>, // Send so thread-safe.
    waker: Once<Waker> // Lazy initialization.
}

impl CoroutineTask {
    // Static so future can live.
    pub fn new(future: impl Future<Output = ()> + Send + Sync + 'static) -> CoroutineTask {
        CoroutineTask {
            future: Box::pin(future),
            waker: Once::new()
        }
    }

    fn poll(&mut self, context: &mut Context) -> Poll<()> {
        self.future.as_mut().poll(context)
    }
}

// Wrapper for intrusive to add RwLock.
struct CoroutineTaskNode {
    link: RBTreeAtomicLink,
    id: TaskId,
    c: RwLock<CoroutineTask>,
}

impl CoroutineTaskNode {
    pub fn new(future: impl Future<Output = ()> + Send + Sync + 'static) -> Self {
        CoroutineTaskNode {
            link: RBTreeAtomicLink::new(),
            id: TaskId::new(),
            c: RwLock::new(CoroutineTask::new(future)),
        }
    }
}

static GLOBAL_EXECUTOR: Once<Executor> = Once::new();

// Intrusive data structures.
intrusive_adapter!(CoroutineTreeAdapter = Arc<CoroutineTaskNode>: CoroutineTaskNode { link => RBTreeAtomicLink });

impl<'a> KeyAdapter<'a> for CoroutineTreeAdapter {
    type Key = &'a TaskId;

    fn get_key(&self, task: &'a CoroutineTaskNode) -> Self::Key {
        &task.id
    }
}

type CoroutineTree = RBTree<CoroutineTreeAdapter>;

fn new_coroutine_tree() -> CoroutineTree {
    CoroutineTree::new(CoroutineTreeAdapter::new())
}

struct CoroutineWaker {
    task_id: TaskId,
    ready_queue: Arc<IntMutex<VecDeque<TaskId>>>,
}

impl CoroutineWaker {
    fn new(task_id: TaskId, ready_queue: Arc<IntMutex<VecDeque<TaskId>>>) -> Waker {
        Waker::from(Arc::new(CoroutineWaker { task_id, ready_queue }))
    }

    fn wake_task(&self) {
        self.ready_queue.lock().push_back(self.task_id);
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
    tasks: IntMutex<CoroutineTree>,
    ready_queue: Arc<IntMutex<VecDeque<TaskId>>>,
}

impl Executor {
    fn new() -> Self {
        Executor {
            tasks: IntMutex::new(new_coroutine_tree()),
            ready_queue: Arc::new(IntMutex::new(VecDeque::new())),
        }
    }

    pub fn run(&self) {
        const BATCH_SIZE: usize = 20;
        loop {
            for _ in 0..BATCH_SIZE {
                let task_id = { self.ready_queue.lock().pop_front() };
                
                let Some(task_id) = task_id else {
                    break; // Empty queue.
                };
                let mut tasks = self.tasks.lock();
                let mut task_cursor = tasks.find_mut(&task_id);

                let should_remove = match task_cursor.get() {
                    Some(task) => {
                        // Get or create waker.
                        {
                            let task_writer = task.c.write();
                            task_writer.waker.call_once(|| CoroutineWaker::new(task.id, Arc::clone(&self.ready_queue)));
                        }
                        // Clone to prevent deadlock with writer.
                        // Are there better ways of doing this?
                        let waker = {
                            let task_reader = task.c.read();
                            task_reader.waker.get().unwrap().clone()
                        };
                        let mut context = Context::from_waker(&waker);

                        match { task.c.write().poll(&mut context) } {
                            Poll::Ready(()) => true, // Done.
                            Poll::Pending => false, // Waker will put in ready queue.
                        }
                    }
                    None => false, // Task not found.
                };

                if should_remove {
                    task_cursor.remove();
                }
            }
            yield_thread();
        }
    }

    fn spawn_node(&self, task: CoroutineTaskNode) {
        let task_id = task.id;
        { self.tasks.lock().insert(Arc::new(task)); }
        { self.ready_queue.lock().push_back(task_id); }
    }

    fn spawn_future(&self, future: impl Future<Output = ()> + Send + Sync + 'static) {
        let task = CoroutineTaskNode::new(future);
        self.spawn_node(task);
    }
}

pub fn spawn_coroutine(future: impl Future<Output = ()> + Send + Sync + 'static) {
    GLOBAL_EXECUTOR.get().unwrap().spawn_future(future);
}

pub fn init_coroutine_queue() {
    GLOBAL_EXECUTOR.call_once(|| Executor::new());
}

pub fn init_coroutine_executor() {
    spawn_thread(|| GLOBAL_EXECUTOR.get().unwrap().run());
}
