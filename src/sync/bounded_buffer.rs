use core::mem::MaybeUninit;

use crate::sync::{IntMutex, MutexLike, Semaphore};

pub struct BoundedBuffer<T, const N: usize> {
    state: IntMutex<BufferState<T, N>>,
    items: Semaphore,  // number of filled slots
    spaces: Semaphore, // number of free slots
}

struct BufferState<T, const N: usize> {
    buffer: [MaybeUninit<T>; N],
    head: usize,
    tail: usize,
    len: usize,
}

impl<T, const N: usize> BoundedBuffer<T, N> {
    pub fn new() -> Self {
        let buffer: [MaybeUninit<T>; N] = [const { MaybeUninit::uninit() }; N];

        BoundedBuffer {
            state: IntMutex::new(BufferState {
                buffer,
                head: 0,
                tail: 0,
                len: 0,
            }),
            items: Semaphore::new(0),
            spaces: Semaphore::new(N),
        }
    }

    // blocks if the buffer is full
    pub fn push(&self, x: T) {
        self.spaces.down();

        {
            let mut st = self.state.lock();
            debug_assert!(st.len < N);

            let tail = st.tail;
            st.buffer[tail].write(x);
            st.tail = (tail + 1) % N;
            st.len += 1;
        }

        self.items.up();
    }

    // blocks if the buffer is empty
    pub fn pop(&self) -> T {
        self.items.down();

        let x = {
            let mut st = self.state.lock();
            debug_assert!(st.len > 0);

            let x = unsafe { st.buffer[st.head].assume_init_read() };
            st.head = (st.head + 1) % N;
            st.len -= 1;
            x
        };

        self.spaces.up();
        x
    }

    pub const fn capacity(&self) -> usize {
        N
    }
}

impl<T, const N: usize> Default for BoundedBuffer<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const N: usize> Drop for BoundedBuffer<T, N> {
    fn drop(&mut self) {
        let mut st = self.state.lock();

        let mut idx = st.head;
        for _ in 0..st.len {
            unsafe {
                st.buffer[idx].assume_init_drop();
            }
            idx = (idx + 1) % N;
        }
    }
}
