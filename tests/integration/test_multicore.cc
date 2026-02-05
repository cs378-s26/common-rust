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

// Test 1: Verify all cores participate
static void test_all_cores_participate() {
  SAY("Test 1: Verify all cores participat\n");

  constexpr int MAX_CORES = 16;
  constexpr int THREADS_PER_CORE = 50;

  Atomic<uint64_t> core_seen[MAX_CORES];
  for (int i = 0; i < MAX_CORES; i++) {
    core_seen[i].set(0);
  }

  CountDownLatch latch{};
  uint64_t total_threads = Sys::core_count * THREADS_PER_CORE;

  for (uint64_t i = 0; i < total_threads; i++) {
    latch.up();
    Thread::create([&core_seen, &latch]() {
      uint64_t id = PerCore::id();
      if (id < MAX_CORES) {
        core_seen[id].set(1);
      }
      Thread::yield();
      latch.down();
    });
    Thread::yield(); // Cooperative scheduling
  }

  latch.sync();

  // Count participating cores
  uint64_t participating = 0;
  for (uint64_t i = 0; i < Sys::core_count && i < MAX_CORES; i++) {
    if (core_seen[i].get() > 0) {
      participating++;
      SAY("  Core ? participated\n", Dec(i));
    }
  }

  TEST_ASSERT_MSG(participating == Sys::core_count, "Not all cores participated");
  SAY("  PASS: All ? cores participated\n", Dec(Sys::core_count));
}

// Test 2: Thread migration detection (this test isn't great, but something)
static void test_thread_migration() {

  SAY("Test 2: Thread migration detection\n");

  constexpr int NUM_THREADS = 500;
  Atomic<uint64_t> migrations{0};
  CountDownLatch latch{};

  for (int i = 0; i < NUM_THREADS; i++) {
    latch.up();
    Thread::create([&migrations, &latch]() {
      uint64_t start_core = PerCore::id();

      // Yield several times to increase migration chance
      for (int j = 0; j < 10; j++) {
        Thread::yield();
      }

      uint64_t end_core = PerCore::id();
      if (start_core != end_core) {
        migrations.fetch_add(1);
      }

      latch.down();
    });
  }

  latch.sync();

  uint64_t migration_count = migrations.get();
  SAY("  ? out of ? threads migrated\n", Dec(migration_count), Dec(NUM_THREADS));

  // With multiple cores, we expect some migrations
  if (Sys::core_count > 1) {
    TEST_ASSERT_MSG(migration_count > 0, "No thread migrations detected with multiple cores");
  }

  SAY("  PASS: Thread migration working\n");
}

// Test 3: Per-core counter isolation
static void test_percore_isolation() {
  SAY("Test 3: Per-core counter isolation\n");

  constexpr int MAX_CORES = 16;
  constexpr int INCREMENTS = 1000;

  Atomic<uint64_t> core_counts[MAX_CORES];
  for (int i = 0; i < MAX_CORES; i++) {
    core_counts[i].set(0);
  }

  CountDownLatch latch{};

  // Create threads that increment their core's counter
  for (int i = 0; i < INCREMENTS; i++) {
    latch.up();
    Thread::create([&core_counts, &latch]() {
      uint64_t id = PerCore::id();
      if (id < MAX_CORES) {
        core_counts[id].fetch_add(1);
      }
      latch.down();
    });
  }

  latch.sync();

  // Sum should equal total increments
  uint64_t total = 0;
  for (uint64_t i = 0; i < Sys::core_count && i < MAX_CORES; i++) {
    uint64_t count = core_counts[i].get();
    total += count;
    SAY("  Core ? count: ?\n", Dec(i), Dec(count));
  }

  TEST_ASSERT_MSG(total == INCREMENTS, "Total count mismatch - race condition?");
  SAY("  PASS: Per-core counters correct (total=?)\n", Dec(total));
}

// Test 4: Load balancing (this test isn't great, but something)
static void test_load_balance() {
  SAY("Test 4: Load balancing\n");

  constexpr int MAX_CORES = 16;
  constexpr int NUM_THREADS = 1000;

  Atomic<uint64_t> core_work[MAX_CORES];
  for (int i = 0; i < MAX_CORES; i++) {
    core_work[i].set(0);
  }

  CountDownLatch latch{};

  for (int i = 0; i < NUM_THREADS; i++) {
    latch.up();
    Thread::create([&core_work, &latch]() {
      // Do some work
      Thread::yield();
      uint64_t id = PerCore::id();
      if (id < MAX_CORES) {
        core_work[id].fetch_add(1);
      }
      Thread::yield();
      latch.down();
    });
    // Interleave yields to encourage balancing
    if (i % 10 == 0) {
      Thread::yield();
    }
  }

  latch.sync();

  // Calculate distribution statistics
  uint64_t total = 0;
  uint64_t min_work = NUM_THREADS;
  uint64_t max_work = 0;

  for (uint64_t i = 0; i < Sys::core_count && i < MAX_CORES; i++) {
    uint64_t work = core_work[i].get();
    total += work;
    if (work < min_work)
      min_work = work;
    if (work > max_work)
      max_work = work;
    SAY("  Core ? did ? work\n", Dec(i), Dec(work));
  }

  TEST_ASSERT_MSG(total == NUM_THREADS, "Total work mismatch");

  // Check that load is reasonably balanced (no core has 0 work with > 1 cores)
  if (Sys::core_count > 1) {
    TEST_ASSERT_MSG(min_work > 0, "Some core did no work");

    // Warn if highly imbalanced (> 10x difference)
    if (max_work > min_work * 10) {
      SAY("  WARNING: High imbalance (min=?, max=?)\n", Dec(min_work), Dec(max_work));
    }
  }

  SAY("  PASS: Load distributed (min=?, max=?)\n", Dec(min_work), Dec(max_work));
}

// Test 5: Stress test (many threads w/ many yields())
static void test_stress() {
  SAY("Test 5: Stress test (many threads w/ many yields())\n");

  constexpr int NUM_THREADS = 10000;
  Atomic<uint64_t> completed{0};
  CountDownLatch latch{};

  for (int i = 0; i < NUM_THREADS; i++) {
    latch.up();
    Thread::create([&completed, &latch]() {
      // Multiple yields per thread
      Thread::yield();
      Thread::yield();
      completed.fetch_add(1);
      latch.down();
    });
  }

  latch.sync();

  TEST_ASSERT_MSG(completed.get() == NUM_THREADS, "Not all threads completed in stress test");
  SAY("  PASS: ? threads completed stress test\n", Dec(NUM_THREADS));
}

// Entry point - called instead of kernel_main
void kernel_main() {
  SAY("=== Multi-Core Integration Tests ===\n");
  SAY("Core count: ?\n", Dec(Sys::core_count));

  if (Sys::core_count == 1) {
    SAY("WARNING: Single-core mode - some tests may be less meaningful\n");
  }

  test_all_cores_participate();
  test_thread_migration();
  test_percore_isolation();
  test_load_balance();
  test_stress();

  SAY("=== All Tests Passed ===\n");
  TEST_PASS();
}
