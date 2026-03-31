use core::ops::DerefMut;

pub mod bounded_buffer;
pub mod int_mutex;
pub mod int_spinlock;
pub mod promise;
pub mod rwlock;
pub mod semaphore;

pub use bounded_buffer::BoundedBuffer;
pub use int_mutex::{IntMutex, IntMutexGuard};
pub use int_spinlock::{IntSpinLock, IntSpinLockGuard};
pub use promise::Promise;
pub use rwlock::{RwLock, RwReadGuard, RwWriteGuard};
pub use semaphore::Semaphore;

pub trait MutexLike<T> {
    type Guard<'a>: DerefMut<Target = T>
    where
        Self: 'a;

    fn lock(&self) -> Self::Guard<'_>;

    fn lock_no_restore_irq(&self) -> Self::Guard<'_>;
}
