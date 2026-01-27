use core::{
    cell::UnsafeCell,
    hint,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicBool, Ordering},
};

use spin::{Mutex, MutexGuard, Once};

use crate::{
    arch::{IrqState, irq_disable},
    thread::{ThreadQueue, can_yield, new_thread_queue, suspend_to_queue},
};

pub struct IntMutexGuard<'a, T> {
    mutex: &'a IntMutex<T>,
    irq_state: IrqState,
}

impl<'a, T> Drop for IntMutexGuard<'a, T> {
    fn drop(&mut self) {
        // TODO: wake things up from the queue
        self.mutex.lock.store(false, Ordering::Release);
        self.irq_state.restore();
    }
}

impl<'a, T> Deref for IntMutexGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.mutex.data.get() }
    }
}

impl<'a, T> DerefMut for IntMutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.mutex.data.get() }
    }
}

/// interrupt-disabled "smart" mutex
/// will spin for a fixed number of cycles before sleeping the current thread, but only if the
/// current context can be preempted.
pub struct IntMutex<T> {
    // underlying mutex
    lock: AtomicBool,
    data: UnsafeCell<T>,
    blocked: Once<Mutex<ThreadQueue>>,
}

pub trait MutexLike<T> {
    type Guard<'a>: DerefMut<Target = T>
    where
        Self: 'a;

    fn lock(&self) -> Self::Guard<'_>;
}

impl<T> IntMutex<T> {
    pub const fn new(init: T) -> IntMutex<T> {
        IntMutex {
            lock: AtomicBool::new(false),
            data: UnsafeCell::new(init),
            blocked: Once::new(),
        }
    }

    #[inline(always)]
    fn attempt_acquire_lock(&self, state: IrqState) -> bool {
        irq_disable();

        if !self.lock.swap(true, Ordering::Acquire) {
            // we got the lock
            return true;
        }

        state.restore();
        false
    }

    #[inline(always)]
    fn lock_block_yield(&self, state: IrqState) {
        while !self.attempt_acquire_lock(state) {
            while self.lock.load(Ordering::Relaxed) {
                // attempt to block
                let queue = self.blocked.call_once(|| Mutex::new(new_thread_queue()));
                irq_disable();
                suspend_to_queue(queue);
            }
        }
    }

    #[inline(always)]
    fn lock_block_spin(&self, state: IrqState) {
        while !self.attempt_acquire_lock(state) {
            while self.lock.load(Ordering::Relaxed) {
                hint::spin_loop();
            }
        }
    }

    #[inline(always)]
    fn lock_block(&self, state: IrqState) {
        if can_yield() {
            self.lock_block_yield(state);
        } else {
            self.lock_block_spin(state);
        }
    }

    pub fn lock(&self) -> IntMutexGuard<'_, T> {
        let state = IrqState::save();

        if !self.attempt_acquire_lock(state) {
            self.lock_block(state);
        }

        IntMutexGuard {
            mutex: self,
            irq_state: state,
        }
    }
}

unsafe impl<T: Send> Send for IntMutex<T> {}
unsafe impl<T: Send> Sync for IntMutex<T> {}

impl<T> MutexLike<T> for IntMutex<T> {
    type Guard<'a>
        = IntMutexGuard<'a, T>
    where
        Self: 'a;

    fn lock(&self) -> Self::Guard<'_> {
        self.lock()
    }
}

impl<T> MutexLike<T> for Mutex<T> {
    type Guard<'a>
        = MutexGuard<'a, T>
    where
        Self: 'a;

    fn lock(&self) -> Self::Guard<'_> {
        self.lock()
    }
}
