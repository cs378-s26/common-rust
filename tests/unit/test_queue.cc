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

// queue.h includes atomic.h which includes machine.h (auto-detects HOST_BUILD)
// Ideally, we don't use ../ but works for right now
#include "../../src/queue.h"

// Test node structure - must have a `next` pointer for Queue (see queue.h)
struct TestNode {
  TestNode *next = nullptr;
  int value;

  explicit TestNode(int v = 0) : value(v) {}
};

class QueueTest : public ::testing::Test {};

// Basic operations with NoLock
TEST_F(QueueTest, EmptyQueueRemove) {
  Queue<TestNode, NoLock> q;
  EXPECT_EQ(q.remove(), nullptr);
  EXPECT_EQ(q.size(), 0);
}

TEST_F(QueueTest, SingleAddRemove) {
  Queue<TestNode, NoLock> q;
  TestNode n(42);

  q.add(&n);
  EXPECT_EQ(q.size(), 1);

  TestNode *result = q.remove();
  EXPECT_EQ(result, &n);
  EXPECT_EQ(result->value, 42);
  EXPECT_EQ(q.size(), 0);
}

TEST_F(QueueTest, FIFOOrder) {
  Queue<TestNode, NoLock> q;
  TestNode n1(1), n2(2), n3(3);

  q.add(&n1);
  q.add(&n2);
  q.add(&n3);

  EXPECT_EQ(q.size(), 3);

  EXPECT_EQ(q.remove()->value, 1);
  EXPECT_EQ(q.remove()->value, 2);
  EXPECT_EQ(q.remove()->value, 3);
  EXPECT_EQ(q.remove(), nullptr);
}

TEST_F(QueueTest, Peek) {
  Queue<TestNode, NoLock> q;
  TestNode n1(1), n2(2);

  EXPECT_EQ(q.peek(), nullptr);

  q.add(&n1);
  EXPECT_EQ(q.peek()->value, 1);
  EXPECT_EQ(q.size(), 1); // Peek doesn't remove

  q.add(&n2);
  EXPECT_EQ(q.peek()->value, 1); // Still first element
}

TEST_F(QueueTest, RemoveAll) {
  Queue<TestNode, NoLock> q;
  TestNode n1(1), n2(2), n3(3);

  q.add(&n1);
  q.add(&n2);
  q.add(&n3);

  TestNode *head = q.remove_all();

  EXPECT_EQ(q.size(), 0);
  EXPECT_EQ(q.remove(), nullptr);

  // Verify the queue is intact
  EXPECT_EQ(head->value, 1);
  EXPECT_EQ(head->next->value, 2);
  EXPECT_EQ(head->next->next->value, 3);
  EXPECT_EQ(head->next->next->next, nullptr);
}

TEST_F(QueueTest, RemoveAllEmpty) {
  Queue<TestNode, NoLock> q;
  EXPECT_EQ(q.remove_all(), nullptr);
  EXPECT_EQ(q.size(), 0);
}

TEST_F(QueueTest, InterleavedAddRemove) {
  Queue<TestNode, NoLock> q;
  TestNode n1(1), n2(2), n3(3), n4(4);

  q.add(&n1);
  q.add(&n2);
  EXPECT_EQ(q.remove()->value, 1);

  q.add(&n3);
  EXPECT_EQ(q.remove()->value, 2);
  EXPECT_EQ(q.remove()->value, 3);

  q.add(&n4);
  EXPECT_EQ(q.remove()->value, 4);
  EXPECT_EQ(q.remove(), nullptr);
}

// Tests with SpinLock
TEST_F(QueueTest, SpinLockBasic) {
  Queue<TestNode, SpinLock> q;
  TestNode n1(1), n2(2);

  q.add(&n1);
  q.add(&n2);

  EXPECT_EQ(q.remove()->value, 1);
  EXPECT_EQ(q.remove()->value, 2);
}

// Concurrent tests
TEST_F(QueueTest, ConcurrentProducerConsumer) {
  Queue<TestNode, SpinLock> q;
  constexpr int NUM_ITEMS = 1000;
  std::atomic<int> consumed{0};

  // Pre-allocate nodes to avoid allocation during test
  std::vector<TestNode> nodes;
  nodes.reserve(NUM_ITEMS);
  for (int i = 0; i < NUM_ITEMS; i++) {
    nodes.emplace_back(i);
  }

  // Producer thread
  std::thread producer([&]() {
    for (int i = 0; i < NUM_ITEMS; i++) {
      q.add(&nodes[i]);
    }
  });

  // Consumer threads
  std::vector<std::thread> consumers;
  for (int i = 0; i < 4; i++) {
    consumers.emplace_back([&]() {
      while (consumed.load() < NUM_ITEMS) {
        TestNode *n = q.remove();
        if (n != nullptr) {
          consumed.fetch_add(1);
        }
      }
    });
  }

  producer.join();
  for (auto &c : consumers) {
    c.join();
  }

  EXPECT_EQ(consumed.load(), NUM_ITEMS);
  EXPECT_EQ(q.size(), 0);
}

TEST_F(QueueTest, MultipleProducers) {
  Queue<TestNode, SpinLock> q;
  constexpr int NUM_PRODUCERS = 4;
  constexpr int ITEMS_PER_PRODUCER = 250;

  // Pre-allocate nodes
  std::vector<std::vector<TestNode>> producer_nodes(NUM_PRODUCERS);
  for (int p = 0; p < NUM_PRODUCERS; p++) {
    producer_nodes[p].reserve(ITEMS_PER_PRODUCER);
    for (int i = 0; i < ITEMS_PER_PRODUCER; i++) {
      producer_nodes[p].emplace_back(p * 1000 + i);
    }
  }

  // Launch producers
  std::vector<std::thread> producers;
  for (int p = 0; p < NUM_PRODUCERS; p++) {
    producers.emplace_back([&, p]() {
      for (int i = 0; i < ITEMS_PER_PRODUCER; i++) {
        q.add(&producer_nodes[p][i]);
      }
    });
  }

  for (auto &p : producers) {
    p.join();
  }

  EXPECT_EQ(q.size(), NUM_PRODUCERS * ITEMS_PER_PRODUCER);

  // Drain and count
  int count = 0;
  while (q.remove() != nullptr) {
    count++;
  }
  EXPECT_EQ(count, NUM_PRODUCERS * ITEMS_PER_PRODUCER);
}

TEST_F(QueueTest, StressTest) {
  Queue<TestNode, SpinLock> q;
  constexpr int NUM_THREADS = 4;
  constexpr int OPS_PER_THREAD = 1000;

  // Each thread has its own nodes
  std::vector<std::vector<TestNode>> thread_nodes(NUM_THREADS);
  for (int t = 0; t < NUM_THREADS; t++) {
    thread_nodes[t].reserve(OPS_PER_THREAD);
    for (int i = 0; i < OPS_PER_THREAD; i++) {
      thread_nodes[t].emplace_back(t * 10000 + i);
    }
  }

  std::atomic<int> total_added{0};
  std::atomic<int> total_removed{0};

  std::vector<std::thread> threads;
  for (int t = 0; t < NUM_THREADS; t++) {
    threads.emplace_back([&, t]() {
      for (int i = 0; i < OPS_PER_THREAD; i++) {
        // Alternate between add and remove
        if (i % 2 == 0) {
          q.add(&thread_nodes[t][i]);
          total_added.fetch_add(1);
        } else {
          if (q.remove() != nullptr) {
            total_removed.fetch_add(1);
          }
        }
      }
    });
  }

  for (auto &t : threads) {
    t.join();
  }

  // Drain remaining items
  while (q.remove() != nullptr) {
    total_removed.fetch_add(1);
  }

  EXPECT_EQ(total_added.load(), total_removed.load());
}
