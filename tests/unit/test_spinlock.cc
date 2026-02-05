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

// we're on the host build so we have stdlib
#include <atomic>
#include <gtest/gtest.h>
#include <thread>
#include <vector>

// atomic.h includes machine.h which auto-detects HOST_BUILD
// Ideally, we don't use ../ but works for right now
#include "../../src/atomic.h"

class SpinLockTest : public ::testing::Test {};

// Basic SpinLock tests
TEST_F(SpinLockTest, LockUnlock) {
  SpinLock lock;
  lock.lock();
  // We hold the lock
  lock.unlock();
  // Lock released
}

TEST_F(SpinLockTest, TryLockSuccess) {
  SpinLock lock;
  EXPECT_TRUE(lock.tryLock());
  lock.unlock();
}

TEST_F(SpinLockTest, TryLockFail) {
  SpinLock lock;
  EXPECT_TRUE(lock.tryLock());
  EXPECT_FALSE(lock.tryLock()); // Already held
  lock.unlock();
  EXPECT_TRUE(lock.tryLock()); // Available again
  lock.unlock();
}

TEST_F(SpinLockTest, MultipleLockUnlock) {
  SpinLock lock;
  for (int i = 0; i < 100; i++) {
    lock.lock();
    lock.unlock();
  }
}

// LockGuard RAII tests
TEST_F(SpinLockTest, LockGuardBasic) {
  SpinLock lock;
  {
    LockGuard<SpinLock> guard(lock);
    EXPECT_FALSE(lock.tryLock()); // Lock is held by guard
  }
  EXPECT_TRUE(lock.tryLock()); // Lock released when guard destroyed
  lock.unlock();
}

TEST_F(SpinLockTest, LockGuardNested) {
  SpinLock lock1, lock2;
  {
    LockGuard<SpinLock> guard1(lock1);
    {
      LockGuard<SpinLock> guard2(lock2);
      EXPECT_FALSE(lock1.tryLock());
      EXPECT_FALSE(lock2.tryLock());
    }
    EXPECT_FALSE(lock1.tryLock());
    EXPECT_TRUE(lock2.tryLock());
    lock2.unlock();
  }
  EXPECT_TRUE(lock1.tryLock());
  lock1.unlock();
}

// LockGuardP tests (pointer version)
TEST_F(SpinLockTest, LockGuardPBasic) {
  SpinLock lock;
  {
    LockGuardP<SpinLock> guard(&lock);
    EXPECT_FALSE(lock.tryLock());
  }
  EXPECT_TRUE(lock.tryLock());
  lock.unlock();
}

TEST_F(SpinLockTest, LockGuardPNull) {
  // Should not crash with null pointer
  LockGuardP<SpinLock> guard(nullptr);
  // No assertions needed - just verify it doesn't crash
}

// Concurrent correctness tests
TEST_F(SpinLockTest, ConcurrentMutualExclusion) {
  SpinLock lock;
  int counter = 0; // Not atomic - protected by lock
  constexpr int NUM_THREADS = 8;
  constexpr int INCREMENTS = 10000;

  std::vector<std::thread> threads;
  for (int i = 0; i < NUM_THREADS; i++) {
    threads.emplace_back([&]() {
      for (int j = 0; j < INCREMENTS; j++) {
        LockGuard<SpinLock> guard(lock);
        counter++;
      }
    });
  }

  for (auto &t : threads) {
    t.join();
  }

  EXPECT_EQ(counter, NUM_THREADS * INCREMENTS);
}

TEST_F(SpinLockTest, ConcurrentReadModifyWrite) {
  SpinLock lock;
  int value = 0;
  constexpr int NUM_THREADS = 4;
  constexpr int ITERATIONS = 5000;

  std::vector<std::thread> threads;
  for (int i = 0; i < NUM_THREADS; i++) {
    threads.emplace_back([&]() {
      for (int j = 0; j < ITERATIONS; j++) {
        LockGuard<SpinLock> guard(lock);
        int temp = value;
        value = temp + 1;
      }
    });
  }

  for (auto &t : threads) {
    t.join();
  }

  EXPECT_EQ(value, NUM_THREADS * ITERATIONS);
}

// SpinBarrier tests
class SpinBarrierTest : public ::testing::Test {};

TEST_F(SpinBarrierTest, SingleThread) {
  SpinBarrier barrier(1);
  barrier.sync();
  // Should return immediately
}

TEST_F(SpinBarrierTest, MultipleThreadsSync) {
  constexpr int NUM_THREADS = 4;
  SpinBarrier barrier(NUM_THREADS);

  std::atomic<int> before_count{0};
  std::atomic<int> after_count{0};

  std::vector<std::thread> threads;
  for (int i = 0; i < NUM_THREADS; i++) {
    threads.emplace_back([&]() {
      before_count.fetch_add(1);
      barrier.sync();
      // At this point, all threads have reached the barrier
      // So before_count should be NUM_THREADS
      EXPECT_EQ(before_count.load(), NUM_THREADS);
      after_count.fetch_add(1);
    });
  }

  for (auto &t : threads) {
    t.join();
  }

  EXPECT_EQ(before_count.load(), NUM_THREADS);
  EXPECT_EQ(after_count.load(), NUM_THREADS);
}

TEST_F(SpinBarrierTest, SynchronizesWork) {
  constexpr int NUM_THREADS = 4;
  SpinBarrier barrier(NUM_THREADS);

  // Each thread sets its flag, then waits, then reads all flags
  std::atomic<bool> flags[NUM_THREADS];
  for (int i = 0; i < NUM_THREADS; i++) {
    flags[i].store(false);
  }

  std::atomic<int> success_count{0};

  std::vector<std::thread> threads;
  for (int i = 0; i < NUM_THREADS; i++) {
    threads.emplace_back([&, i]() {
      // Set my flag
      flags[i].store(true);

      // Wait for all threads
      barrier.sync();

      // Verify all flags are set
      bool all_set = true;
      for (int j = 0; j < NUM_THREADS; j++) {
        if (!flags[j].load()) {
          all_set = false;
        }
      }

      if (all_set) {
        success_count.fetch_add(1);
      }
    });
  }

  for (auto &t : threads) {
    t.join();
  }

  EXPECT_EQ(success_count.load(), NUM_THREADS);
}

// NoLock tests (for completeness)
class NoLockTest : public ::testing::Test {};

TEST_F(NoLockTest, LockUnlock) {
  NoLock lock;
  lock.lock();   // No-op
  lock.unlock(); // No-op
}

TEST_F(NoLockTest, LockGuard) {
  NoLock lock;
  {
    LockGuard<NoLock> guard(lock);
    // Nothing happens, but syntax works
  }
}
