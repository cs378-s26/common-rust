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

#pragma once

#include "atomic.h"

template <typename T, typename LockType> class Queue {
  T *volatile first = nullptr;
  T *volatile last = nullptr;
  LockType lock;
  Atomic<uint64_t> size_count{0};

public:
  Queue() : first(nullptr), last(nullptr), lock() {}
  Queue(const Queue &) = delete;

  void add(T *t) {
    LockGuard g{lock};
    t->next = nullptr;
    if (first == nullptr) {
      first = t;
    } else {
      last->next = t;
    }
    last = t;
    size_count.add_fetch(1);
  }

  T *remove() {
    LockGuard g{lock};
    if (first == nullptr) {
      return nullptr;
    }
    auto it = first;
    first = it->next;
    if (first == nullptr) {
      last = nullptr;
    }
    size_count.add_fetch(-1);
    return it;
  }

  T *remove_all() {
    LockGuard g{lock};
    auto it = first;
    first = nullptr;
    last = nullptr;
    size_count.set(0);
    return it;
  }

  // methods with racey behavior -- why are they here?

  uint64_t size() { return size_count.get(); }

  T *peek() {
    LockGuard g{lock};
    return first;
  }
};
