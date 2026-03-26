use core::{
    cell::UnsafeCell,
    hint,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicBool, Ordering},
};

use crate::{
    arch::{Arch, ArchTrait},
    mp::CoreId,
    state::{Irq, State},
    thread::{
        CORE_PINNED_TO, LOCAL_WORK_QUEUE, PINNED_TO_CORE, ThreadQueue, can_yield, new_thread_queue,
        suspend_to_queue,
    },
};

use super::{MutexLike, int_spinlock::IntSpinLock};

pub struct IntMutexGuard<'a, T> {
    mutex: &'a IntMutex<T>,
    irq_state: State<Irq>,
}

impl<'a, T> Drop for IntMutexGuard<'a, T> {
    fn drop(&mut self) {
        self.mutex.lock.store(false, Ordering::Release);

        if let Some(task) = self.mutex.blocked.lock().pop_front() {
            if PINNED_TO_CORE.read_for(&task).load(Ordering::Relaxed) {
                let core = CoreId(CORE_PINNED_TO.read_for(&task).load(Ordering::Relaxed));
                LOCAL_WORK_QUEUE.read_for(core).lock().push_back(task);
            } else {
                LOCAL_WORK_QUEUE.lock().push_back(task);
            }
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
    fn lock_block_yield(&self, state: State<Irq>) {
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
    fn lock_block_spin(&self, state: State<Irq>) {
        while !self.attempt_acquire_lock(state) {
            while self.lock.load(Ordering::Relaxed) {
                hint::spin_loop();
            }
        }
    }

    #[inline(always)]
    fn lock_block(&self, state: State<Irq>) {
        if can_yield() {
            self.lock_block_yield(state);
        } else {
            self.lock_block_spin(state);
        }
    }

    fn lock_impl(&self, guard_state: State<Irq>) -> IntMutexGuard<'_, T> {
        let state = State::<Irq>::save();

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
        self.lock_impl(State::<Irq>::save())
    }

    fn lock_no_restore_irq(&self) -> Self::Guard<'_> {
        self.lock_impl(State::<Irq>::new(false))
    }
}

unsafe impl<T: Send> Send for IntMutex<T> {}
unsafe impl<T: Send> Sync for IntMutex<T> {}