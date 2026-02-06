use core::{
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
    sync::atomic::{AtomicU64, Ordering},
};

use alloc::task::Wake;
use alloc::boxed::Box;
use alloc::collections::btree_map::BTreeMap;
use alloc::collections::VecDeque;
use alloc::sync::Arc;

use spin::{Once, Mutex};

use crate::{
    thread::{spawn_thread, yield_thread},
    print::kprintln,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct TaskId(u64);

impl TaskId {
    fn new() -> Self {
        // Increment every new instance.
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        // kprintln!("Creating coroutine with ID {}.", NEXT_ID.load(Ordering::SeqCst));
        TaskId(NEXT_ID.fetch_add(1, Ordering::SeqCst))
    }
}

pub struct CoroutineTask {
    id: TaskId,
    future: Pin<Box<dyn Future<Output = ()> + Send>>, // Send so thread-safe.
}

impl CoroutineTask {
    // Static so future can live.
    pub fn new(future: impl Future<Output = ()> + Send + 'static) -> CoroutineTask {
        CoroutineTask {
            id: TaskId::new(),
            future: Box::pin(future),
        }
    }

    fn poll(&mut self, context: &mut Context) -> Poll<()> {
        self.future.as_mut().poll(context)
    }
}

static GLOBAL_EXECUTOR: Once<Executor> = Once::new();

struct CoroutineWaker {
    task_id: TaskId,
    ready_queue: Arc<Mutex<VecDeque<TaskId>>>,
}

impl CoroutineWaker {
    fn new(task_id: TaskId, ready_queue: Arc<Mutex<VecDeque<TaskId>>>) -> Waker {
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
    tasks: Mutex<BTreeMap<TaskId, CoroutineTask>>,
    ready_queue: Arc<Mutex<VecDeque<TaskId>>>,
    waker_cache: Mutex<BTreeMap<TaskId, Waker>>,
}

impl Executor {
    fn new() -> Self {
        Executor {
            tasks: Mutex::new(BTreeMap::new()),
            ready_queue: Arc::new(Mutex::new(VecDeque::new())),
            waker_cache: Mutex::new(BTreeMap::new()),
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
                let mut waker_cache = self.waker_cache.lock();
                
                let task = match tasks.get_mut(&task_id) {
                    Some(task) => task,
                    None => continue, // Task removed already.
                };

                // Get or create waker.
                let waker = waker_cache.entry(task_id)
                    .or_insert_with(|| CoroutineWaker::new(task_id, self.ready_queue.clone()));
                let mut context = Context::from_waker(waker);
                
                match task.poll(&mut context) {
                    Poll::Ready(()) => {
                        // Done.
                        tasks.remove(&task_id);
                        waker_cache.remove(&task_id);
                    }
                    Poll::Pending => {} // Waker will put in ready queue.
                }
            }
            yield_thread();
        }
    }

    fn spawn(&self, task: CoroutineTask) {
        let task_id = task.id;
        { self.tasks.lock().insert(task_id, task); }
        { self.ready_queue.lock().push_back(task_id); }
    }
}

pub fn spawn_coroutine(future: impl Future<Output = ()> + Send + 'static) {
    let task = CoroutineTask::new(future);
    GLOBAL_EXECUTOR.get().unwrap().spawn(task);
}

pub fn init_coroutine_queue() {
    GLOBAL_EXECUTOR.call_once(|| Executor::new());
}

pub fn init_coroutine_executor() {
    spawn_thread(|| GLOBAL_EXECUTOR.get().unwrap().run());
}
