use core::ops::{Deref, DerefMut};

use crate::{
    sync::{IntMutex, IntMutexGuard, MutexLike},
    thread::{
        ThreadQueue, can_yield, is_on_thread, new_thread_queue, schedule_thread,
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

/// Counting semaphore. down() blocks until a permit is available up() releases one.
///
/// If called from a context that cannot yield (like an interrupt handler), down() will
/// spin instead of sleeping. Blocking behavior requires a thread context.
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
        let to_wake;

        {
            let mut st = self.state.lock();
            st.count += 1;
            // wake one waiter so it can grab the new permit
            to_wake = st.waiters.pop_front();
        }

        if let Some(t) = to_wake {
            schedule_thread(t);
        }
    }
}

unsafe impl Send for Semaphore {}
unsafe impl Sync for Semaphore {}
