use core::{
    cell::UnsafeCell,
    hint,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicBool, Ordering},
};

use spin::Mutex;

use crate::{
    arch::{IrqState, irq_disable},
    mp::{MP_STAGE, MPStage},
    thread::{CAN_YIELD, IS_ON_THREAD, ThreadQueue, can_yield, new_thread_queue, yield_thread_with_action},
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
    blocked: Mutex<Option<ThreadQueue>>,
}

impl<T> IntMutex<T> {
    pub const fn new(init: T) -> IntMutex<T> {
        IntMutex {
            lock: AtomicBool::new(false),
            data: UnsafeCell::new(init),
            blocked: Mutex::new(None),
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
                let mut queue = self.blocked.lock();
                irq_disable();

                yield_thread_with_action(|thread| {
                    queue.get_or_insert_with(new_thread_queue).push_back(thread);
                    // drop queue or else bad things happen
                    drop(queue);
                });
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
        if can_yield()        {
            self.lock_block_yield(state);
        } else {
            self.lock_block_spin(state);
        }
    }

    pub fn lock(&self) -> IntMutexGuard<'_, T> {
        let state = IrqState::save();
        irq_disable();

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
