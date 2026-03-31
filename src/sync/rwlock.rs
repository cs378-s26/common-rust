use core::{
    cell::UnsafeCell,
    hint,
    ops::{Deref, DerefMut},
};

use spin::lazy::Lazy;

use crate::{
    sync::{IntMutex, IntMutexGuard, MutexLike},
    thread::{
        ThreadQueue, can_yield, is_on_thread, new_thread_queue, schedule_thread,
        suspend_to_locked_queue,
    },
};

struct RwState {
    readers: usize,
    writer_active: bool,
    waiting_writers: usize,
    read_wait: ThreadQueue,
    write_wait: ThreadQueue,
}

// guard wrappers so suspend_to_locked_queue can enqueue into the right queue
struct ReadWaitGuard<'a>(IntMutexGuard<'a, RwState>);

impl<'a> Deref for ReadWaitGuard<'a> {
    type Target = ThreadQueue;
    fn deref(&self) -> &Self::Target {
        &self.0.read_wait
    }
}

impl<'a> DerefMut for ReadWaitGuard<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0.read_wait
    }
}

struct WriteWaitGuard<'a>(IntMutexGuard<'a, RwState>);

impl<'a> Deref for WriteWaitGuard<'a> {
    type Target = ThreadQueue;
    fn deref(&self) -> &Self::Target {
        &self.0.write_wait
    }
}

impl<'a> DerefMut for WriteWaitGuard<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0.write_wait
    }
}

/// Reader-writer lock. Multiple readers can hold the lock simultaneously writers get
/// exclusive access.
///
/// This is writer-preferring: once a writer is waiting, new readers will block until
/// the writer has gone through. This prevents writers from starving behind a steady
/// stream of readers.
pub struct RwLock<T> {
    data: UnsafeCell<T>,
    state: Lazy<IntMutex<RwState>>,
}

impl<T> RwLock<T> {
    pub const fn new(value: T) -> Self {
        RwLock {
            data: UnsafeCell::new(value),
            state: Lazy::new(|| {
                IntMutex::new(RwState {
                    readers: 0,
                    writer_active: false,
                    waiting_writers: 0,
                    read_wait: new_thread_queue(),
                    write_wait: new_thread_queue(),
                })
            }),
        }
    }

    // readers only enter if no active writer and no waiting writers
    pub fn read_lock(&self) -> RwReadGuard<'_, T> {
        loop {
            let mut st = self.state.lock();

            if !st.writer_active && st.waiting_writers == 0 {
                st.readers += 1;
                return RwReadGuard { lock: self };
            }

            if !can_yield() {
                drop(st);
                // spin safely
                while {
                    let st = self.state.lock();
                    st.writer_active || st.waiting_writers != 0
                } {
                    hint::spin_loop();
                }
                continue;
            }

            assert!(
                is_on_thread(),
                "RwLock::read_lock() must be called from thread context when blocking"
            );

            // sleep: enqueue ourselves while holding the state lock, then drop it inside
            suspend_to_locked_queue(ReadWaitGuard(st));
        }
    }

    pub fn write_lock(&self) -> RwWriteGuard<'_, T> {
        let mut registered_waiter = false;

        loop {
            let mut st = self.state.lock();

            if !st.writer_active && st.readers == 0 {
                st.writer_active = true;
                if registered_waiter {
                    st.waiting_writers -= 1;
                }
                return RwWriteGuard { lock: self };
            }

            if !registered_waiter {
                st.waiting_writers += 1;
                registered_waiter = true;
            }

            if !can_yield() {
                drop(st);
                // spin until available
                while {
                    let st = self.state.lock();
                    st.writer_active || st.readers != 0
                } {
                    hint::spin_loop();
                }
                continue;
            }

            assert!(
                is_on_thread(),
                "RwLock::write_lock() must be called from thread context when blocking"
            );

            // sleep: enqueue ourselves while holding the state lock, then drop it inside
            suspend_to_locked_queue(WriteWaitGuard(st));
        }
    }

    fn read_unlock(&self) {
        let mut to_wake = None;

        {
            let mut st = self.state.lock();
            debug_assert!(st.readers > 0);
            st.readers -= 1;

            if st.readers == 0 && st.waiting_writers > 0 {
                to_wake = st.write_wait.pop_front();
            }
        }

        if let Some(t) = to_wake {
            schedule_thread(t);
        }
    }

    fn write_unlock(&self) {
        let mut wake_one_writer = None;
        let mut wake_readers: ThreadQueue = new_thread_queue();

        {
            let mut st = self.state.lock();
            debug_assert!(st.writer_active);
            st.writer_active = false;

            if st.waiting_writers > 0 {
                wake_one_writer = st.write_wait.pop_front();
            } else {
                while let Some(t) = st.read_wait.pop_front() {
                    schedule_thread(t);
                }
            }
        }

        if let Some(t) = wake_one_writer {
            schedule_thread(t);
        }

        while let Some(t) = wake_readers.pop_front() {
            schedule_thread(t);
        }
    }
}

pub struct RwReadGuard<'a, T> {
    lock: &'a RwLock<T>,
}

impl<'a, T> Deref for RwReadGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // safe because readers hold the lock writers are excluded
        unsafe { &*self.lock.data.get() }
    }
}

impl<'a, T> Drop for RwReadGuard<'a, T> {
    fn drop(&mut self) {
        self.lock.read_unlock();
    }
}

pub struct RwWriteGuard<'a, T> {
    lock: &'a RwLock<T>,
}

impl<'a, T> Deref for RwWriteGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // safe because writer holds exclusive access
        unsafe { &*self.lock.data.get() }
    }
}

impl<'a, T> DerefMut for RwWriteGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<'a, T> Drop for RwWriteGuard<'a, T> {
    fn drop(&mut self) {
        self.lock.write_unlock();
    }
}

unsafe impl<T: Send> Send for RwLock<T> {}
unsafe impl<T: Send + Sync> Sync for RwLock<T> {}
