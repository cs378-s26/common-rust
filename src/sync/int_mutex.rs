use core::{
    cell::UnsafeCell,
    hint,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicBool, Ordering},
};

use crate::{
    arch::{Arch, ArchTrait, IrqState},
    thread::{ThreadQueue, can_yield, local_work_queue, new_thread_queue, suspend_to_queue},
};

use super::{MutexLike, int_spinlock::IntSpinLock};

pub struct IntMutexGuard<'a, T> {
    mutex: &'a IntMutex<T>,
    irq_state: IrqState,
}

impl<'a, T> Drop for IntMutexGuard<'a, T> {
    fn drop(&mut self) {
        self.mutex.lock.store(false, Ordering::Release);

        if let Some(task) = self.mutex.blocked.lock().pop_front() {
            local_work_queue().push_back(task);
        }

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
    blocked: IntSpinLock<ThreadQueue>,
}

impl<T> IntMutex<T> {
    pub const fn new(init: T) -> IntMutex<T> {
        IntMutex {
            lock: AtomicBool::new(false),
            data: UnsafeCell::new(init),
            blocked: IntSpinLock::new(new_thread_queue()),
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
                let queue = &self.blocked;
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

    fn lock_impl(&self, guard_state: IrqState) -> IntMutexGuard<'_, T> {
        let state = IrqState::save();

        if !self.attempt_acquire_lock(state) {
            self.lock_block(state);
        }

        IntMutexGuard {
            mutex: self,
            irq_state: guard_state,
        }
    }
}

impl<T> MutexLike<T> for IntMutex<T> {
    type Guard<'a>
        = IntMutexGuard<'a, T>
    where
        Self: 'a;

    fn lock(&self) -> Self::Guard<'_> {
        self.lock_impl(IrqState::save())
    }

    fn lock_no_restore_irq(&self) -> Self::Guard<'_> {
        self.lock_impl(IrqState::new(false))
    }
}

unsafe impl<T: Send> Send for IntMutex<T> {}
unsafe impl<T: Send> Sync for IntMutex<T> {}
