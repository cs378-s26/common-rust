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

#include "machine.h"

template <typename T> class AtomicPtr {
  volatile T *ptr;

public:
  AtomicPtr() : ptr(nullptr) {}
  AtomicPtr(T *x) : ptr(x) {}
  AtomicPtr<T> &operator=(T v) {
    __atomic_store_n(ptr, v, __ATOMIC_SEQ_CST);
    return *this;
  }
  operator T() const { return __atomic_load_n(ptr, __ATOMIC_SEQ_CST); }
  T fetch_add(T inc) { return __atomic_fetch_add(ptr, inc, __ATOMIC_SEQ_CST); }
  T add_fetch(T inc) { return __atomic_add_fetch(ptr, inc, __ATOMIC_SEQ_CST); }
  void set(T inc) { return __atomic_store_n(ptr, inc, __ATOMIC_SEQ_CST); }
  T get(void) { return __atomic_load_n(ptr, __ATOMIC_SEQ_CST); }
  T exchange(T v) {
    T ret;
    __atomic_exchange(ptr, &v, &ret, __ATOMIC_SEQ_CST);
    return ret;
  }
};

template <typename T> class Atomic {
  volatile T value;

public:
  Atomic() : value(0) {}
  Atomic(T x) : value(x) {}
  Atomic<T> &operator=(T v) {
    __atomic_store_n(&value, v, __ATOMIC_SEQ_CST);
    return *this;
  }
  operator T() const { return __atomic_load_n(&value, __ATOMIC_SEQ_CST); }
  T fetch_add(T inc) {
    return __atomic_fetch_add(&value, inc, __ATOMIC_SEQ_CST);
  }
  T add_fetch(T inc) {
    return __atomic_add_fetch(&value, inc, __ATOMIC_SEQ_CST);
  }
  void set(T inc) { return __atomic_store_n(&value, inc, __ATOMIC_SEQ_CST); }
  T get(void) { return __atomic_load_n(&value, __ATOMIC_SEQ_CST); }
  T exchange(T v) {
    T ret;
    __atomic_exchange(&value, &v, &ret, __ATOMIC_SEQ_CST);
    return ret;
  }
};

template <typename T> class LockGuard {
  T &it;

public:
  inline LockGuard(T &it) : it(it) { it.lock(); }
  inline ~LockGuard() { it.unlock(); }
};

template <typename T> class LockGuardP {
  T *it;

public:
  inline LockGuardP(T *it) : it(it) {
    if (it)
      it->lock();
  }
  inline ~LockGuardP() {
    if (it)
      it->unlock();
  }
};

class NoLock {
public:
  inline void lock() {}
  inline void unlock() {}
};

inline void stackInALoop() { pause(); }

class SpinLock {
  Atomic<bool> taken{false};

public:
  SpinLock() {}
  SpinLock(const SpinLock &) = delete;
  SpinLock &operator=(const SpinLock &) = delete;

  bool tryLock() { return !taken.exchange(true); }

  void lock() {
    while (!tryLock()) {
      stackInALoop();
    }
  }

  void unlock() { taken.set(false); }
};

class SpinBarrier {
  Atomic<uint64_t> count;

public:
  SpinBarrier(uint64_t count) : count(count) {}
  void sync() {
    count.add_fetch(uint64_t(-1));
    while (count.get() > 0) {
      stackInALoop();
    }
  }
};
