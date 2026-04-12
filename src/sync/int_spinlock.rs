use core::{
    cell::UnsafeCell,
    hint,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicBool, Ordering},
};

use super::MutexLike;
use crate::{
    arch::{Arch, ArchTrait},
    state::{Irq, State},
};

pub struct IntSpinLockGuard<'a, T> {
    mutex: &'a IntSpinLock<T>,
    irq_state: State<Irq>,
}

impl<'a, T> Drop for IntSpinLockGuard<'a, T> {
    fn drop(&mut self) {
        self.mutex.lock.store(false, Ordering::Release);
        self.irq_state.restore();
    }
}

impl<'a, T> Deref for IntSpinLockGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.mutex.data.get() }
    }
}

impl<'a, T> DerefMut for IntSpinLockGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.mutex.data.get() }
    }
}

pub struct IntSpinLock<T> {
    lock: AtomicBool,
    data: UnsafeCell<T>,
}

impl<T> IntSpinLock<T> {
    pub const fn new(init: T) -> IntSpinLock<T> {
        IntSpinLock {
            lock: AtomicBool::new(false),
            data: UnsafeCell::new(init),
        }
    }

    #[inline(always)]
    fn attempt_acquire_lock(&self, state: State<Irq>) -> bool {
        Arch::set_irq_enabled(false);

        if !self.lock.swap(true, Ordering::Acquire) {
            // we got the lock
            return true;
        }

        state.restore();
        false
    }

    #[inline(always)]
    fn lock_block(&self, state: State<Irq>) {
        while !self.attempt_acquire_lock(state) {
            while self.lock.load(Ordering::Relaxed) {
                hint::spin_loop();
            }
        }
    }

    fn lock_impl(&self, guard_state: State<Irq>) -> IntSpinLockGuard<'_, T> {
        let state = State::<Irq>::save();

        if !self.attempt_acquire_lock(state) {
            self.lock_block(state);
        }

        IntSpinLockGuard {
            mutex: self,
            irq_state: guard_state,
        }
    }
}

unsafe impl<T: Send> Send for IntSpinLock<T> {}
unsafe impl<T: Send> Sync for IntSpinLock<T> {}

impl<T> MutexLike<T> for IntSpinLock<T> {
    type Guard<'a>
        = IntSpinLockGuard<'a, T>
    where
        Self: 'a;

    fn lock(&self) -> Self::Guard<'_> {
        self.lock_impl(State::<Irq>::save())
    }

    fn lock_no_restore_irq(&self) -> Self::Guard<'_> {
        self.lock_impl(State::<Irq>::new(false))
    }
}
