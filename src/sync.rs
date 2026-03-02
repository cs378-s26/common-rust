use core::{
    cell::UnsafeCell,
    hint,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicBool, Ordering},
};

use spin::{Mutex, MutexGuard, lazy::Lazy};

use crate::{
    arch::{Arch, ArchTrait, IrqState, IrqStateTrait},
    thread::{ThreadQueue, can_yield, local_work_queue, new_thread_queue, suspend_to_queue},
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

        if let Some(task) = self.mutex.blocked.lock().pop_front() {
            local_work_queue().push_back(task);
        }
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
    blocked: Lazy<Mutex<ThreadQueue>>,
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
            blocked: Lazy::new(|| Mutex::new(new_thread_queue())),
        }
    }

    #[inline(always)]
    fn attempt_acquire_lock(&self, state: IrqState) -> bool {
        Arch::set_irq_enabled(false);

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
            // TODO: don't hardcode this constant
            for _ in 0..500 {
                if !self.lock.load(Ordering::Relaxed) {
                    break;
                }

                hint::spin_loop();
            }

            while self.lock.load(Ordering::Relaxed) {
                // attempt to block
                let queue = &*self.blocked;
                Arch::set_irq_enabled(false);
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
