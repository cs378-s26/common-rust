#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_common::test_runner)]

kernel_common::integration_test!({
    extern crate alloc;
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicUsize, Ordering};
    use kernel_common::print::kprintln;
    use kernel_common::sync::{BoundedBuffer, Promise, RwLock, Semaphore};
    use kernel_common::thread::{spawn_thread, yield_thread};

    // // ── constants ────────────────────────────────────────────────────────────────

    const THREADS: usize = 4;
    const STRESS_THREADS: usize = 16;
    const ITERS: usize = 100;
    const STRESS_ITERS: usize = 500;

    // ── semaphore ────────────────────────────────────────────────────────────────

    // Basic: 4 producers each up() 100 times, 4 consumers each down() 100 times.
    // Verifies that no permits are created or lost.
    {
        let sem = Arc::new(Semaphore::new(0));
        let latch = Arc::new(AtomicUsize::new(0));

        for _ in 0..THREADS {
            let sem = sem.clone();
            let latch = latch.clone();
            spawn_thread(move || {
                for _ in 0..ITERS {
                    sem.up();
                }
                latch.fetch_add(1, Ordering::SeqCst);
            });
        }

        for _ in 0..THREADS {
            let sem = sem.clone();
            let latch = latch.clone();
            spawn_thread(move || {
                for _ in 0..ITERS {
                    sem.down();
                }
                latch.fetch_add(1, Ordering::SeqCst);
            });
        }

        while latch.load(Ordering::SeqCst) != THREADS * 2 {}
        kprintln!("semaphore basic test passed");
    }

    // Stress: 16 producers and 16 consumers at 500 iterations each.
    // Heavy contention on the internal state lock.
    {
        let sem = Arc::new(Semaphore::new(0));
        let latch = Arc::new(AtomicUsize::new(0));

        for _ in 0..STRESS_THREADS {
            let sem = sem.clone();
            let latch = latch.clone();
            spawn_thread(move || {
                for _ in 0..STRESS_ITERS {
                    sem.up();
                }
                latch.fetch_add(1, Ordering::SeqCst);
            });
        }

        for _ in 0..STRESS_THREADS {
            let sem = sem.clone();
            let latch = latch.clone();
            spawn_thread(move || {
                for _ in 0..STRESS_ITERS {
                    sem.down();
                }
                latch.fetch_add(1, Ordering::SeqCst);
            });
        }

        while latch.load(Ordering::SeqCst) != STRESS_THREADS * 2 {}
        kprintln!("semaphore stress test passed");
    }

    // ── promise ──────────────────────────────────────────────────────────────────

    // Basic: 4 threads block on get(), then set() wakes them all.
    {
        let promise: Arc<Promise<u64>> = Arc::new(Promise::new());
        let latch = Arc::new(AtomicUsize::new(0));

        for _ in 0..THREADS {
            let promise = promise.clone();
            let latch = latch.clone();
            spawn_thread(move || {
                let v = promise.get();
                assert!(v == 42, "promise returned wrong value");
                latch.fetch_add(1, Ordering::SeqCst);
            });
        }

        yield_thread();
        promise.set(42u64);

        while latch.load(Ordering::SeqCst) != THREADS {}
        kprintln!("promise basic test passed");
    }

    // Many waiters: 16 threads all block, set() must wake all of them.
    {
        let promise: Arc<Promise<u64>> = Arc::new(Promise::new());
        let latch = Arc::new(AtomicUsize::new(0));

        for _ in 0..STRESS_THREADS {
            let promise = promise.clone();
            let latch = latch.clone();
            spawn_thread(move || {
                let v = promise.get();
                assert!(v == 7, "promise returned wrong value");
                latch.fetch_add(1, Ordering::SeqCst);
            });
        }

        yield_thread();
        promise.set(7u64);

        while latch.load(Ordering::SeqCst) != STRESS_THREADS {}
        kprintln!("promise many waiters test passed");
    }

    // Already-set: set() is called before any thread calls get().
    // Exercises the fast path where get() returns immediately without blocking.
    {
        let promise: Arc<Promise<u64>> = Arc::new(Promise::new());
        promise.set(99u64);

        let latch = Arc::new(AtomicUsize::new(0));

        for _ in 0..THREADS {
            let promise = promise.clone();
            let latch = latch.clone();
            spawn_thread(move || {
                let v = promise.get();
                assert!(v == 99, "promise returned wrong value");
                latch.fetch_add(1, Ordering::SeqCst);
            });
        }

        while latch.load(Ordering::SeqCst) != THREADS {}
        kprintln!("promise already set test passed");
    }

    // ── bounded buffer ───────────────────────────────────────────────────────────

    // Basic: 4 producers push 1..=25 each, 4 consumers pop 25 each.
    // Verifies sum = 4 * (1+2+...+25) = 1300.
    {
        let buf: Arc<BoundedBuffer<u64, 8>> = Arc::new(BoundedBuffer::new());
        let sum = Arc::new(AtomicUsize::new(0));
        let latch = Arc::new(AtomicUsize::new(0));

        for _ in 0..THREADS {
            let buf = buf.clone();
            let latch = latch.clone();
            spawn_thread(move || {
                for i in 1u64..=25 {
                    buf.push(i);
                }
                latch.fetch_add(1, Ordering::SeqCst);
            });
        }

        for _ in 0..THREADS {
            let buf = buf.clone();
            let sum = sum.clone();
            let latch = latch.clone();
            spawn_thread(move || {
                for _ in 0..25 {
                    sum.fetch_add(buf.pop() as usize, Ordering::SeqCst);
                }
                latch.fetch_add(1, Ordering::SeqCst);
            });
        }

        while latch.load(Ordering::SeqCst) != THREADS * 2 {}
        assert!(
            sum.load(Ordering::SeqCst) == 1300,
            "bounded buffer lost items"
        );
        kprintln!("bounded buffer basic test passed");
    }

    // Stress: buffer capacity 2 with 4 producers/consumers pushing 1..=50 each.
    // Very tight buffer forces near-constant blocking on both sides.
    // Verifies sum = 4 * (1+2+...+50) = 5100.
    {
        let buf: Arc<BoundedBuffer<u64, 2>> = Arc::new(BoundedBuffer::new());
        let sum = Arc::new(AtomicUsize::new(0));
        let latch = Arc::new(AtomicUsize::new(0));

        for _ in 0..THREADS {
            let buf = buf.clone();
            let latch = latch.clone();
            spawn_thread(move || {
                for i in 1u64..=50 {
                    buf.push(i);
                }
                latch.fetch_add(1, Ordering::SeqCst);
            });
        }

        for _ in 0..THREADS {
            let buf = buf.clone();
            let sum = sum.clone();
            let latch = latch.clone();
            spawn_thread(move || {
                for _ in 0..50 {
                    sum.fetch_add(buf.pop() as usize, Ordering::SeqCst);
                }
                latch.fetch_add(1, Ordering::SeqCst);
            });
        }

        while latch.load(Ordering::SeqCst) != THREADS * 2 {}
        assert!(
            sum.load(Ordering::SeqCst) == 5100,
            "bounded buffer lost items"
        );
        kprintln!("bounded buffer stress test passed");
    }

    // Single-slot: capacity 1, 4 producers and 4 consumers.
    // Every push blocks until the slot is consumed — maximally tight.
    // Verifies sum = 4 * (1+2+...+10) = 220.
    {
        let buf: Arc<BoundedBuffer<u64, 1>> = Arc::new(BoundedBuffer::new());
        let sum = Arc::new(AtomicUsize::new(0));
        let latch = Arc::new(AtomicUsize::new(0));

        for _ in 0..THREADS {
            let buf = buf.clone();
            let latch = latch.clone();
            spawn_thread(move || {
                for i in 1u64..=10 {
                    buf.push(i);
                }
                latch.fetch_add(1, Ordering::SeqCst);
            });
        }

        for _ in 0..THREADS {
            let buf = buf.clone();
            let sum = sum.clone();
            let latch = latch.clone();
            spawn_thread(move || {
                for _ in 0..10 {
                    sum.fetch_add(buf.pop() as usize, Ordering::SeqCst);
                }
                latch.fetch_add(1, Ordering::SeqCst);
            });
        }

        while latch.load(Ordering::SeqCst) != THREADS * 2 {}
        assert!(
            sum.load(Ordering::SeqCst) == 220,
            "bounded buffer lost items"
        );
        kprintln!("bounded buffer single slot test passed");
    }

    // ── rwlock ───────────────────────────────────────────────────────────────────

    // Basic: 4 writers each increment 100 times, 4 readers exercise the read path.
    // Verifies final value = 4 * 100 = 400.
    {
        let lock: Arc<RwLock<u64>> = Arc::new(RwLock::new(0));
        let latch = Arc::new(AtomicUsize::new(0));

        for _ in 0..THREADS {
            let lock = lock.clone();
            let latch = latch.clone();
            spawn_thread(move || {
                for _ in 0..ITERS {
                    let mut g = lock.write_lock();
                    *g += 1;
                }
                latch.fetch_add(1, Ordering::SeqCst);
            });
        }

        for _ in 0..THREADS {
            let lock = lock.clone();
            let latch = latch.clone();
            spawn_thread(move || {
                for _ in 0..ITERS {
                    let _g = lock.read_lock();
                }
                latch.fetch_add(1, Ordering::SeqCst);
            });
        }

        while latch.load(Ordering::SeqCst) != THREADS * 2 {}
        assert!(
            *lock.read_lock() == (THREADS * ITERS) as u64,
            "rwlock write count wrong"
        );
        kprintln!("rwlock basic test passed");
    }

    // Reader-heavy: 12 readers and 2 writers running concurrently.
    // Stresses simultaneous read access and writer-starvation prevention.
    // Verifies final value = 2 * 100 = 200.
    {
        const READERS: usize = 12;
        const WRITERS: usize = 2;

        let lock: Arc<RwLock<u64>> = Arc::new(RwLock::new(0));
        let latch = Arc::new(AtomicUsize::new(0));

        for _ in 0..WRITERS {
            let lock = lock.clone();
            let latch = latch.clone();
            spawn_thread(move || {
                for _ in 0..ITERS {
                    let mut g = lock.write_lock();
                    *g += 1;
                }
                latch.fetch_add(1, Ordering::SeqCst);
            });
        }

        for _ in 0..READERS {
            let lock = lock.clone();
            let latch = latch.clone();
            spawn_thread(move || {
                for _ in 0..ITERS {
                    let _g = lock.read_lock();
                }
                latch.fetch_add(1, Ordering::SeqCst);
            });
        }

        while latch.load(Ordering::SeqCst) != READERS + WRITERS {}
        assert!(
            *lock.read_lock() == (WRITERS * ITERS) as u64,
            "rwlock write count wrong"
        );
        kprintln!("rwlock reader heavy test passed");
    }

    // Write-heavy stress: 8 writers competing for exclusive access at 500 iters each.
    // Maximum writer contention — verifies exclusion and exact final count.
    // Verifies final value = 8 * 500 = 4000.
    {
        const WRITERS: usize = 8;

        let lock: Arc<RwLock<u64>> = Arc::new(RwLock::new(0));
        let latch = Arc::new(AtomicUsize::new(0));

        for _ in 0..WRITERS {
            let lock = lock.clone();
            let latch = latch.clone();
            spawn_thread(move || {
                for _ in 0..STRESS_ITERS {
                    let mut g = lock.write_lock();
                    *g += 1;
                }
                latch.fetch_add(1, Ordering::SeqCst);
            });
        }

        while latch.load(Ordering::SeqCst) != WRITERS {}
        assert!(
            *lock.read_lock() == (WRITERS * STRESS_ITERS) as u64,
            "rwlock write count wrong"
        );
        kprintln!("rwlock write stress test passed");
    }
});

// #![no_std]
// #![no_main]
// #![feature(decl_macro)]
// #![feature(const_trait_impl)]
// #![feature(const_default)]
// #![feature(slice_ptr_get)]
// #![feature(box_as_ptr)]
// #![feature(const_range)]
// #![feature(never_type)]
// #![feature(sync_unsafe_cell)]
// #![feature(custom_test_frameworks)]
// #![test_runner(kernel_common::test_runner)]
// #![reexport_test_harness_main = "test_main"]

// extern crate alloc;

// use alloc::sync::Arc;
// use core::sync::atomic::{AtomicUsize, Ordering};

// use kernel_common::MP_REQUEST;
// use kernel_common::arch::{Arch, ArchTrait, KernelEntryTrait};
// use kernel_common::coroutine::{init_coroutine_executor, init_coroutine_queue};
// use kernel_common::mp::{MP_STAGE, MPStage};
// use kernel_common::print::kprintln;
// use kernel_common::sync::{BoundedBuffer, Promise, RwLock, Semaphore};
// use kernel_common::thread::{poll_tasks, set_up_idle, spawn_thread, yield_thread};
// use spin::{Barrier, Once};

// #[cfg(test)]
// static INIT_THREADING_BARRIER: Once<Barrier> = Once::new();
// #[cfg(test)]
// static MP_PREEMPT_ENTER_BARRIER: Once<Barrier> = Once::new();
// #[cfg(test)]
// static MAKE_TEST_THREAD: Once<()> = Once::new();

// #[cfg(test)]
// pub struct TestKernelEntry;

// #[cfg(test)]
// impl KernelEntryTrait for TestKernelEntry {
//     fn kernel_main() -> ! {
//         let mp_res = MP_REQUEST
//             .get_response()
//             .expect("Expected to find MpResponse, found None.");
//         let core_count = mp_res.cpus().len();

//         INIT_THREADING_BARRIER
//             .call_once(|| {
//                 init_coroutine_queue();
//                 Barrier::new(core_count)
//             })
//             .wait();

//         set_up_idle();

//         init_coroutine_executor();

//         MP_PREEMPT_ENTER_BARRIER
//             .call_once(|| Barrier::new(core_count))
//             .wait();

//         MP_STAGE.store(MPStage::MPPreempt, Ordering::SeqCst);

//         MAKE_TEST_THREAD.call_once(|| {
//             spawn_thread(move || {
//                 kprintln!("Starting Testing Code...");
//                 crate::test_main();
//             })
//         });

//         Arch::set_irq_enabled(true);
//         poll_tasks()
//     }
// }

// #[unsafe(no_mangle)]
// unsafe extern "C" fn system_main() -> ! {
//     kernel_common::system_init::<Arch, TestKernelEntry>();
// }

// #[panic_handler]
// fn rust_panic(info: &core::panic::PanicInfo) -> ! {
//     kernel_common::test_utils::rust_panic_test_impl(info);
// }

// // ── constants ────────────────────────────────────────────────────────────────

// const THREADS: usize = 4;
// const STRESS_THREADS: usize = 16;
// const ITERS: usize = 100;
// const STRESS_ITERS: usize = 500;

// // ── semaphore ────────────────────────────────────────────────────────────────

// // Basic: 4 producers each up() 100 times, 4 consumers each down() 100 times.
// // Verifies that no permits are created or lost.
// #[test_case]
// fn test_semaphore_basic() {
//     let sem = Arc::new(Semaphore::new(0));
//     let latch = Arc::new(AtomicUsize::new(0));

//     for _ in 0..THREADS {
//         let sem = sem.clone();
//         let latch = latch.clone();
//         spawn_thread(move || {
//             for _ in 0..ITERS {
//                 sem.up();
//             }
//             latch.fetch_add(1, Ordering::SeqCst);
//         });
//     }

//     for _ in 0..THREADS {
//         let sem = sem.clone();
//         let latch = latch.clone();
//         spawn_thread(move || {
//             for _ in 0..ITERS {
//                 sem.down();
//             }
//             latch.fetch_add(1, Ordering::SeqCst);
//         });
//     }

//     while latch.load(Ordering::SeqCst) != THREADS * 2 {}
//     kprintln!("semaphore basic test passed");
// }

// // Stress: 16 producers and 16 consumers at 500 iterations each.
// // Heavy contention on the internal state lock.
// #[test_case]
// fn test_semaphore_stress() {
//     let sem = Arc::new(Semaphore::new(0));
//     let latch = Arc::new(AtomicUsize::new(0));

//     for _ in 0..STRESS_THREADS {
//         let sem = sem.clone();
//         let latch = latch.clone();
//         spawn_thread(move || {
//             for _ in 0..STRESS_ITERS {
//                 sem.up();
//             }
//             latch.fetch_add(1, Ordering::SeqCst);
//         });
//     }

//     for _ in 0..STRESS_THREADS {
//         let sem = sem.clone();
//         let latch = latch.clone();
//         spawn_thread(move || {
//             for _ in 0..STRESS_ITERS {
//                 sem.down();
//             }
//             latch.fetch_add(1, Ordering::SeqCst);
//         });
//     }

//     while latch.load(Ordering::SeqCst) != STRESS_THREADS * 2 {}
//     kprintln!("semaphore stress test passed");
// }

// // ── promise ──────────────────────────────────────────────────────────────────

// // Basic: 4 threads block on get(), then set() wakes them all.
// #[test_case]
// fn test_promise_basic() {
//     let promise: Arc<Promise<u64>> = Arc::new(Promise::new());
//     let latch = Arc::new(AtomicUsize::new(0));

//     for _ in 0..THREADS {
//         let promise = promise.clone();
//         let latch = latch.clone();
//         spawn_thread(move || {
//             let v = promise.get();
//             assert!(v == 42, "promise returned wrong value");
//             latch.fetch_add(1, Ordering::SeqCst);
//         });
//     }

//     yield_thread();
//     promise.set(42u64);

//     while latch.load(Ordering::SeqCst) != THREADS {}
//     kprintln!("promise basic test passed");
// }

// // Many waiters: 16 threads all block, set() must wake all of them.
// #[test_case]
// fn test_promise_many_waiters() {
//     let promise: Arc<Promise<u64>> = Arc::new(Promise::new());
//     let latch = Arc::new(AtomicUsize::new(0));

//     for _ in 0..STRESS_THREADS {
//         let promise = promise.clone();
//         let latch = latch.clone();
//         spawn_thread(move || {
//             let v = promise.get();
//             assert!(v == 7, "promise returned wrong value");
//             latch.fetch_add(1, Ordering::SeqCst);
//         });
//     }

//     yield_thread();
//     promise.set(7u64);

//     while latch.load(Ordering::SeqCst) != STRESS_THREADS {}
//     kprintln!("promise many waiters test passed");
// }

// // Already-set: set() is called before any thread calls get().
// // Exercises the fast path where get() returns immediately without blocking.
// #[test_case]
// fn test_promise_already_set() {
//     let promise: Arc<Promise<u64>> = Arc::new(Promise::new());
//     promise.set(99u64);

//     let latch = Arc::new(AtomicUsize::new(0));

//     for _ in 0..THREADS {
//         let promise = promise.clone();
//         let latch = latch.clone();
//         spawn_thread(move || {
//             let v = promise.get();
//             assert!(v == 99, "promise returned wrong value");
//             latch.fetch_add(1, Ordering::SeqCst);
//         });
//     }

//     while latch.load(Ordering::SeqCst) != THREADS {}
//     kprintln!("promise already set test passed");
// }

// // ── bounded buffer ───────────────────────────────────────────────────────────

// // Basic: 4 producers push 1..=25 each, 4 consumers pop 25 each.
// // Verifies sum = 4 * (1+2+...+25) = 1300.
// #[test_case]
// fn test_bounded_buffer_basic() {
//     let buf: Arc<BoundedBuffer<u64, 8>> = Arc::new(BoundedBuffer::new());
//     let sum = Arc::new(AtomicUsize::new(0));
//     let latch = Arc::new(AtomicUsize::new(0));

//     for _ in 0..THREADS {
//         let buf = buf.clone();
//         let latch = latch.clone();
//         spawn_thread(move || {
//             for i in 1u64..=25 {
//                 buf.push(i);
//             }
//             latch.fetch_add(1, Ordering::SeqCst);
//         });
//     }

//     for _ in 0..THREADS {
//         let buf = buf.clone();
//         let sum = sum.clone();
//         let latch = latch.clone();
//         spawn_thread(move || {
//             for _ in 0..25 {
//                 sum.fetch_add(buf.pop() as usize, Ordering::SeqCst);
//             }
//             latch.fetch_add(1, Ordering::SeqCst);
//         });
//     }

//     while latch.load(Ordering::SeqCst) != THREADS * 2 {}
//     assert!(
//         sum.load(Ordering::SeqCst) == 1300,
//         "bounded buffer lost items"
//     );
//     kprintln!("bounded buffer basic test passed");
// }

// // Stress: buffer capacity 2 with 4 producers/consumers pushing 1..=50 each.
// // Very tight buffer forces near-constant blocking on both sides.
// // Verifies sum = 4 * (1+2+...+50) = 5100.
// #[test_case]
// fn test_bounded_buffer_stress() {
//     let buf: Arc<BoundedBuffer<u64, 2>> = Arc::new(BoundedBuffer::new());
//     let sum = Arc::new(AtomicUsize::new(0));
//     let latch = Arc::new(AtomicUsize::new(0));

//     for _ in 0..THREADS {
//         let buf = buf.clone();
//         let latch = latch.clone();
//         spawn_thread(move || {
//             for i in 1u64..=50 {
//                 buf.push(i);
//             }
//             latch.fetch_add(1, Ordering::SeqCst);
//         });
//     }

//     for _ in 0..THREADS {
//         let buf = buf.clone();
//         let sum = sum.clone();
//         let latch = latch.clone();
//         spawn_thread(move || {
//             for _ in 0..50 {
//                 sum.fetch_add(buf.pop() as usize, Ordering::SeqCst);
//             }
//             latch.fetch_add(1, Ordering::SeqCst);
//         });
//     }

//     while latch.load(Ordering::SeqCst) != THREADS * 2 {}
//     assert!(
//         sum.load(Ordering::SeqCst) == 5100,
//         "bounded buffer lost items"
//     );
//     kprintln!("bounded buffer stress test passed");
// }

// // Single-slot: capacity 1, 4 producers and 4 consumers.
// // Every push blocks until the slot is consumed — maximally tight.
// // Verifies sum = 4 * (1+2+...+10) = 220.
// #[test_case]
// fn test_bounded_buffer_single_slot() {
//     let buf: Arc<BoundedBuffer<u64, 1>> = Arc::new(BoundedBuffer::new());
//     let sum = Arc::new(AtomicUsize::new(0));
//     let latch = Arc::new(AtomicUsize::new(0));

//     for _ in 0..THREADS {
//         let buf = buf.clone();
//         let latch = latch.clone();
//         spawn_thread(move || {
//             for i in 1u64..=10 {
//                 buf.push(i);
//             }
//             latch.fetch_add(1, Ordering::SeqCst);
//         });
//     }

//     for _ in 0..THREADS {
//         let buf = buf.clone();
//         let sum = sum.clone();
//         let latch = latch.clone();
//         spawn_thread(move || {
//             for _ in 0..10 {
//                 sum.fetch_add(buf.pop() as usize, Ordering::SeqCst);
//             }
//             latch.fetch_add(1, Ordering::SeqCst);
//         });
//     }

//     while latch.load(Ordering::SeqCst) != THREADS * 2 {}
//     assert!(
//         sum.load(Ordering::SeqCst) == 220,
//         "bounded buffer lost items"
//     );
//     kprintln!("bounded buffer single slot test passed");
// }

// // ── rwlock ───────────────────────────────────────────────────────────────────

// // Basic: 4 writers each increment 100 times, 4 readers exercise the read path.
// // Verifies final value = 4 * 100 = 400.
// #[test_case]
// fn test_rwlock_basic() {
//     let lock: Arc<RwLock<u64>> = Arc::new(RwLock::new(0));
//     let latch = Arc::new(AtomicUsize::new(0));

//     for _ in 0..THREADS {
//         let lock = lock.clone();
//         let latch = latch.clone();
//         spawn_thread(move || {
//             for _ in 0..ITERS {
//                 let mut g = lock.write_lock();
//                 *g += 1;
//             }
//             latch.fetch_add(1, Ordering::SeqCst);
//         });
//     }

//     for _ in 0..THREADS {
//         let lock = lock.clone();
//         let latch = latch.clone();
//         spawn_thread(move || {
//             for _ in 0..ITERS {
//                 let _g = lock.read_lock();
//             }
//             latch.fetch_add(1, Ordering::SeqCst);
//         });
//     }

//     while latch.load(Ordering::SeqCst) != THREADS * 2 {}
//     assert!(
//         *lock.read_lock() == (THREADS * ITERS) as u64,
//         "rwlock write count wrong"
//     );
//     kprintln!("rwlock basic test passed");
// }

// // Reader-heavy: 12 readers and 2 writers running concurrently.
// // Stresses simultaneous read access and writer-starvation prevention.
// // Verifies final value = 2 * 100 = 200.
// #[test_case]
// fn test_rwlock_reader_heavy() {
//     const READERS: usize = 12;
//     const WRITERS: usize = 2;

//     let lock: Arc<RwLock<u64>> = Arc::new(RwLock::new(0));
//     let latch = Arc::new(AtomicUsize::new(0));

//     for _ in 0..WRITERS {
//         let lock = lock.clone();
//         let latch = latch.clone();
//         spawn_thread(move || {
//             for _ in 0..ITERS {
//                 let mut g = lock.write_lock();
//                 *g += 1;
//             }
//             latch.fetch_add(1, Ordering::SeqCst);
//         });
//     }

//     for _ in 0..READERS {
//         let lock = lock.clone();
//         let latch = latch.clone();
//         spawn_thread(move || {
//             for _ in 0..ITERS {
//                 let _g = lock.read_lock();
//             }
//             latch.fetch_add(1, Ordering::SeqCst);
//         });
//     }

//     while latch.load(Ordering::SeqCst) != READERS + WRITERS {}
//     assert!(
//         *lock.read_lock() == (WRITERS * ITERS) as u64,
//         "rwlock write count wrong"
//     );
//     kprintln!("rwlock reader heavy test passed");
// }

// // Write-heavy stress: 8 writers competing for exclusive access at 500 iters each.
// // Maximum writer contention — verifies exclusion and exact final count.
// // Verifies final value = 8 * 500 = 4000.
// #[test_case]
// fn test_rwlock_write_stress() {
//     const WRITERS: usize = 8;

//     let lock: Arc<RwLock<u64>> = Arc::new(RwLock::new(0));
//     let latch = Arc::new(AtomicUsize::new(0));

//     for _ in 0..WRITERS {
//         let lock = lock.clone();
//         let latch = latch.clone();
//         spawn_thread(move || {
//             for _ in 0..STRESS_ITERS {
//                 let mut g = lock.write_lock();
//                 *g += 1;
//             }
//             latch.fetch_add(1, Ordering::SeqCst);
//         });
//     }

//     while latch.load(Ordering::SeqCst) != WRITERS {}
//     assert!(
//         *lock.read_lock() == (WRITERS * STRESS_ITERS) as u64,
//         "rwlock write count wrong"
//     );
//     kprintln!("rwlock write stress test passed");
// }
