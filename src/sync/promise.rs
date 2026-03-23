use core::ops::{Deref, DerefMut};

use spin::lazy::Lazy;

use crate::{
    sync::{IntMutex, IntMutexGuard, MutexLike},
    thread::{
        ThreadQueue, can_yield, is_on_thread, local_work_queue, new_thread_queue,
        suspend_to_locked_queue,
    },
};

struct PromiseWaitGuard<'a, T>(IntMutexGuard<'a, PromiseState<T>>);

impl<'a, T> Deref for PromiseWaitGuard<'a, T> {
    type Target = ThreadQueue;

    fn deref(&self) -> &Self::Target {
        &self.0.waiters
    }
}

impl<'a, T> DerefMut for PromiseWaitGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0.waiters
    }
}

struct PromiseState<T> {
    value: Option<T>,
    waiters: ThreadQueue,
}

pub struct Promise<T> {
    state: Lazy<IntMutex<PromiseState<T>>>,
}

impl<T> Promise<T> {
    pub const fn new() -> Self {
        Promise {
            state: Lazy::new(|| {
                IntMutex::new(PromiseState {
                    value: None,
                    waiters: new_thread_queue(),
                })
            }),
        }
    }

    /// set the promise exactly once and wake all waiters
    pub fn set(&self, x: T) {
        let mut to_wake: ThreadQueue = new_thread_queue();

        {
            let mut st = self.state.lock();

            assert!(st.value.is_none(), "Promise::set() called more than once");
            st.value = Some(x);

            // wake everyone waiting
            while let Some(t) = st.waiters.pop_front() {
                to_wake.push_back(t);
            }
        }

        // woken threads back to the local work queue.
        while let Some(t) = to_wake.pop_front() {
            local_work_queue().push_back(t);
        }
    }
}

impl<T: Clone> Promise<T> {
    // block until the promise is set
    // then return a clone of the value
    pub fn get(&self) -> T {
        loop {
            let st = self.state.lock();

            if let Some(v) = st.value.as_ref() {
                return v.clone();
            }

            // not ready yet
            if !can_yield() {
                // can't block here, so spin safely (we relock for safety)
                drop(st);
                while self.state.lock().value.is_none() {
                    core::hint::spin_loop();
                }
                continue;
            }

            assert!(
                is_on_thread(),
                "Promise::get() must be called from thread context when blocking"
            );

            // sleep: enqueue ourselves while holding the lock, then drop it inside
            suspend_to_locked_queue(PromiseWaitGuard(st));

            // when we wake, retry loop.
        }
    }
}

unsafe impl<T: Send> Send for Promise<T> {}
unsafe impl<T: Send> Sync for Promise<T> {}
