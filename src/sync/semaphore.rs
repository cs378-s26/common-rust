use core::ops::{Deref, DerefMut};

use crate::{
    sync::{IntMutex, IntMutexGuard, MutexLike},
    thread::{
        ThreadQueue, can_yield, is_on_thread, local_work_queue, new_thread_queue,
        suspend_to_locked_queue,
    },
};

struct SemWaitGuard<'a>(IntMutexGuard<'a, SemState>);

impl<'a> Deref for SemWaitGuard<'a> {
    type Target = ThreadQueue;

    fn deref(&self) -> &Self::Target {
        &self.0.waiters
    }
}

impl<'a> DerefMut for SemWaitGuard<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0.waiters
    }
}

// state of the semaphore in a struct so i can easily put lock around it
struct SemState {
    count: usize,
    waiters: ThreadQueue,
}

pub struct Semaphore {
    state: IntMutex<SemState>,
}

impl Semaphore {
    pub fn new(initial: usize) -> Self {
        Semaphore {
            state: IntMutex::new(SemState {
                count: initial,
                waiters: new_thread_queue(),
            }),
        }
    }

    pub fn down(&self) {
        loop {
            let mut st = self.state.lock();

            if st.count > 0 {
                st.count -= 1;
                return;
            }

            if !can_yield() {
                // can't block: drop lock and spin until a permit exists
                drop(st);
                while self.state.lock().count == 0 {
                    core::hint::spin_loop();
                }
                continue;
            }

            assert!(
                is_on_thread(),
                "Semaphore::down() must be called from thread context when blocking"
            );

            // sleep: enqueue ourselves while holding the lock, then drop the lock inside
            suspend_to_locked_queue(SemWaitGuard(st));

            // when we wake, retry loop
        }
    }

    pub fn up(&self) {
        let mut to_wake = None;

        {
            let mut st = self.state.lock();

            if let Some(t) = st.waiters.pop_front() {
                // directly hand the permit to a waiter by waking it
                to_wake = Some(t);
            } else {
                // no one waiting increase available permits
                st.count += 1;
            }
        }

        if let Some(t) = to_wake {
            local_work_queue().push_back(t);
        }
    }
}

unsafe impl Send for Semaphore {}
unsafe impl Sync for Semaphore {}
