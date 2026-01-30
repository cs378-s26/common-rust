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
#include <gtest/gtest.h>
#include <thread>
#include <vector>

// atomic.h includes machine.h which auto-detects HOST_BUILD
// Ideally, we don't use ../ but works for right now
#include "../../src/atomic.h"

class AtomicTest : public ::testing::Test {};

// Basic Atomic<T> operations
TEST_F(AtomicTest, DefaultConstructor) {
    Atomic<uint64_t> a;
    EXPECT_EQ(a.get(), 0);
}

TEST_F(AtomicTest, ValueConstructor) {
    Atomic<uint64_t> a(42);
    EXPECT_EQ(a.get(), 42);
}

TEST_F(AtomicTest, SetAndGet) {
    Atomic<uint64_t> a;
    a.set(100);
    EXPECT_EQ(a.get(), 100);
    a.set(200);
    EXPECT_EQ(a.get(), 200);
}

TEST_F(AtomicTest, Assignment) {
    Atomic<uint64_t> a;
    a = 55;
    EXPECT_EQ(a.get(), 55);
}

TEST_F(AtomicTest, ImplicitConversion) {
    Atomic<uint64_t> a(77);
    uint64_t val = a;  // implicit conversion via operator T()
    EXPECT_EQ(val, 77);
}

TEST_F(AtomicTest, FetchAdd) {
    Atomic<uint64_t> a(10);
    uint64_t old = a.fetch_add(5);
    EXPECT_EQ(old, 10);
    EXPECT_EQ(a.get(), 15);
}

TEST_F(AtomicTest, AddFetch) {
    Atomic<uint64_t> a(10);
    uint64_t newval = a.add_fetch(5);
    EXPECT_EQ(newval, 15);
    EXPECT_EQ(a.get(), 15);
}

TEST_F(AtomicTest, Exchange) {
    Atomic<uint64_t> a(100);
    uint64_t old = a.exchange(200);
    EXPECT_EQ(old, 100);
    EXPECT_EQ(a.get(), 200);
}

TEST_F(AtomicTest, FetchAddNegative) {
    Atomic<uint64_t> a(100);
    a.fetch_add(uint64_t(-10));  // Subtract 10
    EXPECT_EQ(a.get(), 90);
}

// AtomicPtr<T> tests
TEST_F(AtomicTest, AtomicPtrBasic) {
    uint64_t storage = 0;
    AtomicPtr<uint64_t> ptr(&storage);

    ptr = 42;
    EXPECT_EQ(storage, 42);

    uint64_t val = ptr;  // implicit conversion
    EXPECT_EQ(val, 42);
}

TEST_F(AtomicTest, AtomicPtrFetchAdd) {
    uint64_t storage = 10;
    AtomicPtr<uint64_t> ptr(&storage);

    uint64_t old = ptr.fetch_add(5);
    EXPECT_EQ(old, 10);
    EXPECT_EQ(storage, 15);
}

TEST_F(AtomicTest, AtomicPtrExchange) {
    uint64_t storage = 100;
    AtomicPtr<uint64_t> ptr(&storage);

    uint64_t old = ptr.exchange(200);
    EXPECT_EQ(old, 100);
    EXPECT_EQ(storage, 200);
}

// Concurrent tests
TEST_F(AtomicTest, ConcurrentIncrement) {
    Atomic<uint64_t> counter(0);
    constexpr int NUM_THREADS = 8;
    constexpr int INCREMENTS_PER_THREAD = 10000;

    std::vector<std::thread> threads;
    for (int i = 0; i < NUM_THREADS; i++) {
        threads.emplace_back([&counter]() {
            for (int j = 0; j < INCREMENTS_PER_THREAD; j++) {
                counter.fetch_add(1);
            }
        });
    }

    for (auto& t : threads) {
        t.join();
    }
    long long expected = NUM_THREADS * INCREMENTS_PER_THREAD;
    EXPECT_EQ(counter.get(), expected);
}

TEST_F(AtomicTest, ConcurrentExchange) {
    Atomic<uint64_t> value(0);
    constexpr int NUM_THREADS = 4;
    constexpr int ITERATIONS = 1000;

    Atomic<uint64_t> sum(0);

    std::vector<std::thread> threads;
    for (int i = 0; i < NUM_THREADS; i++) {
        threads.emplace_back([&value, &sum, i]() {
            for (int j = 0; j < ITERATIONS; j++) {
                // Each thread tries to set its ID and accumulates old values
                uint64_t old = value.exchange(i + 1);
                sum.fetch_add(old);
            }
        });
    }

    for (auto& t : threads) {
        t.join();
    }

    // The final value should be one of the thread IDs (1-4)
    uint64_t final_val = value.get();
    EXPECT_GE(final_val, 1);
    EXPECT_LE(final_val, NUM_THREADS);
}
