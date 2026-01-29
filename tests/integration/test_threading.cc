/* Copyright (C) 2025 Ahmed Gheith and contributors.
 *
 * Use restricted to classroom projects.
 *
 * THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
 * WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
 * MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR ANY
 * SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
 * WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN ACTION
 * OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF OR IN
 * CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
 */

#include "harness.h"

// Ideally, we don't use ../ but works for right now
#include "../../src/atomic.h"
#include "../../src/count_down_latch.h"
#include "../../src/per_core.h"
#include "../../src/print.h"
#include "../../src/system_main.h"
#include "../../src/thread.h"

// Test 1: Basic thread creation and completion
static void test_basic_threading() {
    SAY("Test 1: Basic thread creation\n");

    constexpr int NUM_THREADS = 100;
    Atomic<uint64_t> counter{0};
    CountDownLatch latch{};

    for (int i = 0; i < NUM_THREADS; i++) {
        latch.up();
        Thread::create([&counter, &latch]() {
            counter.fetch_add(1);
            latch.down();
        });
    }

    latch.sync();

    TEST_ASSERT_MSG(counter.get() == NUM_THREADS,
                    "Not all threads completed");
    SAY("  PASS: ? threads completed\n", Dec(NUM_THREADS));
}

// Test 2: Thread yield behavior 
static void test_yield() {
    SAY("Test 2: Thread yield behavior\n");

    constexpr int NUM_THREADS = 50;
    Atomic<uint64_t> yield_count{0};
    CountDownLatch latch{};

    for (int i = 0; i < NUM_THREADS; i++) {
        latch.up();
        Thread::create([&yield_count, &latch]() {
            // Yield multiple times per thread
            for (int j = 0; j < 5; j++) {
                Thread::yield();
            }
            yield_count.fetch_add(1);
            latch.down();
        });
    }

    latch.sync();

    TEST_ASSERT_MSG(yield_count.get() == NUM_THREADS,
                    "Not all threads completed after yield");
    SAY("  PASS: ? threads yielded successfully\n", Dec(NUM_THREADS));
}

// Test 3: Verify work happens on multiple cores (this test isn't great, but something)
static void test_multicore_work() {
    SAY("Test 3: Verify work happens on multiple cores\n");

    constexpr int NUM_THREADS = 200;
    constexpr int MAX_CORES = 16;

    Atomic<uint64_t> core_work[MAX_CORES];
    for (int i = 0; i < MAX_CORES; i++) {
        core_work[i].set(0);
    }

    CountDownLatch latch{};

    for (int i = 0; i < NUM_THREADS; i++) {
        latch.up();
        Thread::create([&core_work, &latch]() {
            uint64_t core_id = PerCore::id();
            if (core_id < MAX_CORES) {
                core_work[core_id].fetch_add(1);
            }
            Thread::yield();
            latch.down();
        });
        // Yield to encourage distribution
        Thread::yield();
    }

    latch.sync();

    // Count cores that did work
    int active_cores = 0;
    uint64_t total_work = 0;
    for (uint64_t i = 0; i < Sys::core_count && i < MAX_CORES; i++) {
        uint64_t work = core_work[i].get();
        total_work += work;
        if (work > 0) {
            active_cores++;
            SAY("  Core ? did ? units of work\n", Dec(i), Dec(work));
        }
    }

    TEST_ASSERT_MSG(total_work == NUM_THREADS,
                    "Total work doesn't match thread count");

    // With multiple cores, at least 2 should have done work
    if (Sys::core_count > 1) {
        TEST_ASSERT_MSG(active_cores >= 2,
                        "Work not distributed across cores");
    }

    SAY("  PASS: Work distributed across ? cores\n", Dec(active_cores));
}

// Test 4: CountDownLatch correctness
static void test_latch() {
    SAY("Test 4: CountDownLatch correctness\n");

    constexpr int NUM_THREADS = 100;
    Atomic<uint64_t> phase1_count{0};
    Atomic<uint64_t> phase2_count{0};
    CountDownLatch latch1{};
    CountDownLatch latch2{};

    for (int i = 0; i < NUM_THREADS; i++) {
        latch1.up();
        latch2.up();
        Thread::create([&]() {
            phase1_count.fetch_add(1);
            latch1.down();

            // Brief pause
            for (int j = 0; j < 10; j++) {
                Thread::yield();
            }

            phase2_count.fetch_add(1);
            latch2.down();
        });
    }

    latch1.sync();
    uint64_t p1 = phase1_count.get();
    TEST_ASSERT_MSG(p1 == NUM_THREADS, "Phase 1 incomplete");

    latch2.sync();
    uint64_t p2 = phase2_count.get();
    TEST_ASSERT_MSG(p2 == NUM_THREADS, "Phase 2 incomplete");

    SAY("  PASS: Latch synchronization correct\n");
}

// Entry point - called instead of kernel_main
void kernel_main() {
    SAY("=== Threading Integration Tests ===\n");
    SAY("Core count: ?\n", Dec(Sys::core_count));

    test_basic_threading();
    test_yield();
    test_multicore_work();
    test_latch();

    SAY("=== All Tests Passed ===\n");
    TEST_PASS();
}
