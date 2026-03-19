#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_common::test_runner)]

kernel_common::integration_test!({
    use core::sync::atomic::{AtomicUsize, Ordering};
    use kernel_common::print::kprintln;
    use kernel_common::sync::{IntMutex, MutexLike};
    use kernel_common::thread::{spawn_thread, yield_thread};

    fn work(i: u64) -> u64 {
        let mut sum = 0;
        for j in 1..(1 << i) {
            sum += j;
        }
        sum
    }

    const THREADS: usize = 16;
    static LATCH: AtomicUsize = AtomicUsize::new(0);
    const UPPER: usize = 23; // precisely tuned value for runtime
    static VALUES: IntMutex<[u64; UPPER]> = IntMutex::new([0; UPPER]);
    // stress preemption, yielding, and blocking interactions
    kprintln!("spawning busyworking threads");
    for _ in 0..THREADS {
        spawn_thread(|| {
            for i in 3..UPPER {
                let value = work(i as u64);
                let mut lock = VALUES.lock();
                if lock[i] == 0 {
                    lock[i] = value;
                    kprintln!("work({}): {}", i, value); // deliberately inside lock to try to encourage blocking
                }
                drop(lock);
                yield_thread();
            }
            LATCH.fetch_add(1, Ordering::SeqCst);
        });
        yield_thread();
    }
    while LATCH.load(Ordering::SeqCst) != THREADS {} // spin
    LATCH.store(0, Ordering::SeqCst);

    // are we actually preempting?
    kprintln!("spawning infinite loop threads");
    for i in 0..THREADS {
        spawn_thread(|| {
            loop {
                LATCH.fetch_add(1, Ordering::SeqCst);
                let value = work(48); // should never actually finish
                kprintln!("{}", value); // appease linter
            }
        });
        if i == 0 {
            kprintln!("spawned first thread")
        };
        yield_thread();
        if i == 0 {
            kprintln!("returned from yielding to first thread")
        };
    }
    while LATCH.load(Ordering::SeqCst) != THREADS {} // spin
    kprintln!("yay we seem to have actually preempted");

    kprintln!("test complete!");
});
