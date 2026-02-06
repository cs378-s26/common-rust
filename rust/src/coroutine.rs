use core::{
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker, RawWaker, RawWakerVTable},
};
use alloc::boxed::Box;

use alloc::collections::VecDeque;
use spin::{Once, Mutex};

use crate::{
    thread::{spawn_thread, yield_thread},
    print::kprintln,
    arch::core_count,
};


pub struct CoroutineTask {
    future: Pin<Box<dyn Future<Output = ()> + Send>>, // Send so thread-safe.
}

impl CoroutineTask {
    // Static so future can live.
    pub fn new(future: impl Future<Output = ()> + Send + 'static) -> CoroutineTask {
        CoroutineTask {
            future: Box::pin(future),
        }
    }

    fn poll(&mut self, context: &mut Context) -> Poll<()> {
        self.future.as_mut().poll(context)
    }
}

// Ready queue for coroutines.
pub type CoroutineQueue = VecDeque<CoroutineTask>;
// TODO: Do we need interrupts disabled? Not sure if coroutines would die.
static GLOBAL_COROUTINE_QUEUE: Once<Mutex<CoroutineQueue>> = Once::new();

pub fn spawn_coroutine(future: impl Future<Output = ()> + Send + 'static) {
    let task = CoroutineTask::new(future);
    GLOBAL_COROUTINE_QUEUE.get().unwrap().lock().push_back(task);
}

// Wakers that do nothing.
fn dummy_raw_waker() -> RawWaker {
    fn no_op(_: *const ()) {}
    fn clone(_: *const ()) -> RawWaker {
        dummy_raw_waker() // Just make a new one.
    }

    // All no-op functions.
    let vtable = &RawWakerVTable::new(clone, no_op, no_op, no_op);

    RawWaker::new(0 as *const (), vtable)
}

fn dummy_waker() -> Waker {
    unsafe { Waker::from_raw(dummy_raw_waker()) }
}

fn coroutine_executor() {
    loop {
        let task = { GLOBAL_COROUTINE_QUEUE.get().unwrap().lock().pop_front() };
        if let Some(mut task) = task {
            let waker = dummy_waker();
            let mut context = Context::from_waker(&waker);

            match task.poll(&mut context) {
                Poll::Ready(()) => {} // Done.
                Poll::Pending => {
                    // Put back in queue.
                    // TODO: use waker.
                    GLOBAL_COROUTINE_QUEUE.get().unwrap().lock().push_back(task);
                }
            }
        }
        yield_thread();
    }
}

pub fn init_coroutine_queue() {
    GLOBAL_COROUTINE_QUEUE.call_once(|| Mutex::new(VecDeque::new()));
}

pub fn init_coroutine_executor() {
    spawn_thread(coroutine_executor);
}
