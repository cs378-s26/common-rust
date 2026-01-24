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

#include "heap.h"
#include "atomic.h"
#include "print.h"
#include <cstdint>

namespace impl {

uint64_t n_malloc{0};
uint64_t n_free{0};
uint64_t n_leak{0};

static uint64_t *array;
static uint64_t len;
static bool safe = false;
static uint64_t avail = 0; // index 0 will not be available by design
SpinLock *theLock = nullptr;

void makeTaken(uint64_t i, uint64_t entries);
void makeAvail(uint64_t i, uint64_t entries);

inline bool isTaken(uint64_t i) { return array[i] & 1; }
inline bool isAvail(uint64_t i) { return !(array[i] & 1); }
inline uint64_t size(uint64_t i) { return array[i] & ~uint64_t(1); }

uint64_t headerFromFooter(uint64_t i) { return i - size(i) + 1; }

uint64_t footerFromHeader(uint64_t i) { return i + size(i) - 1; }

uint64_t sanity(uint64_t i) {
  if (safe) {
    if (i == 0)
      return 0;
    if (i >= len) {
      KPANIC("bad header index ?\n", i);
    }
    uint64_t footer = footerFromHeader(i);
    if (footer >= len) {
      KPANIC("bad footer index ?\n", footer);
    }
    const auto hv = array[i];
    const auto fv = array[footer];

    if (hv != fv) {
      KPANIC("bad block at ?, hv:? fv:?\n", i, hv, fv);
    }
  }

  return i;
}

uint64_t left(uint64_t i) { return sanity(headerFromFooter(i - 1)); }

uint64_t right(uint64_t i) { return sanity(i + size(i)); }

uint64_t next(uint64_t i) { return sanity(array[i + 1]); }

uint64_t prev(uint64_t i) { return sanity(array[i + 2]); }

void setNext(uint64_t i, uint64_t x) { array[i + 1] = x; }

void setPrev(uint64_t i, uint64_t x) { array[i + 2] = x; }

void remove(uint64_t i) {
  uint64_t prevIndex = prev(i);
  uint64_t nextIndex = next(i);

  if (prevIndex == 0) {
    /* at head */
    avail = nextIndex;
  } else {
    /* in the middle */
    setNext(prevIndex, nextIndex);
  }
  if (nextIndex != 0) {
    setPrev(nextIndex, prevIndex);
  }
}

void makeAvail(uint64_t i, uint64_t entry_count) {
  ASSERT((entry_count & 1) == 0);
  array[i] = entry_count;
  array[footerFromHeader(i)] = entry_count;
  setNext(i, avail);
  setPrev(i, 0);
  if (avail != 0) {
    setPrev(avail, i);
  }
  avail = i;
}

void makeTaken(uint64_t i, uint64_t entry_count) {
  ASSERT((entry_count & 1) == 0);
  array[i] = entry_count + 1;
  array[footerFromHeader(i)] = entry_count + 1;
}

void check_leaks() {
  LockGuardP g{theLock};
  if (n_free + n_leak != n_malloc) {
    SAY("heap leaks: (n_free:?+n_leak:?)==? != n_malloc:?\n", Dec(n_free),
        Dec(n_leak), Dec(n_free + n_leak), Dec(n_malloc));
  }
}

}; // namespace impl

namespace heap {
void init(uintptr_t base, uint64_t bytes) {
  using namespace impl;

  KPRINT("| heap init ? ?\n", base, bytes);

  const uintptr_t alignedBase = (base + 8 + 15) / 16 * 16 - 8;
  ASSERT((alignedBase % 16) == 8);
  ASSERT(alignedBase >= base);
  const auto delta = alignedBase - base;
  ASSERT(delta < 16);

  base = alignedBase;

  ASSERT(bytes >= delta);
  bytes -= delta;

  bytes = bytes / 16 * 16;
  ASSERT((bytes % 16) == 0);

  ASSERT(bytes >
         64); // 16 (start marker) + 32 (one available node) + 16 (end marker)

  KDEBUG("| heap range ? ?\n", base, base + bytes);

  /* can't say new becasue we're initializing the heap */
  array = (uint64_t *)base;

  len = bytes / 8;
  makeTaken(0, 2);
  makeAvail(2, len - 4);
  makeTaken(len - 2, 2);
  theLock = leak(new SpinLock(), true);
}

} // namespace heap

void *malloc(std::size_t bytes) noexcept {
  using namespace impl;
  // Debug::printf("malloc(%d)\n",bytes);
  if (bytes == 0)
    return (void *)array;

  uint64_t entries = ((bytes + 7) / 8) + 2;
  if (entries < 4)
    entries = 4;

  if (entries & 1) {
    entries++;
  }

  LockGuardP g{theLock};

  void *res = 0;

  uint64_t mx = UINT64_MAX;
  uint64_t it = 0;

  {
    int countDown = 20;
    uint64_t p = avail;
    while (p != 0) {
      if (isTaken(p)) {
        KPANIC("block is taken in malloc ?\n", p);
      }
      uint64_t sz = size(p);

      if (sz >= entries) {
        if (sz < mx) {
          mx = sz;
          it = p;
        }
        countDown--;
        if ((countDown == 0) || (sz == entries))
          break;
      }
      p = next(p);
    }
  }

  if (it != 0) {
    remove(it);
    int extra = mx - entries;
    if (extra >= 4) {
      makeTaken(it, entries);
      makeAvail(it + entries, extra);
    } else {
      makeTaken(it, mx);
    }
    res = &array[it + 1];
  }

  if (res != nullptr) {
    n_malloc += 1;
  }
  return res;
}

void free(void *p) noexcept {
  using namespace impl;
  if (p == 0)
    return;
  if (p == (void *)array)
    return;

  LockGuardP g{theLock};

  n_free += 1;
  int idx = ((((uintptr_t)p) - ((uintptr_t)array)) / 8) - 1;
  sanity(idx);
  if (isAvail(idx)) {
    KDEBUG("freeing free block, p:? idx:?\n", p, idx);
  }

  int sz = size(idx);

  int leftIndex = left(idx);
  int rightIndex = right(idx);

  if (isAvail(leftIndex)) {
    remove(leftIndex);
    idx = leftIndex;
    sz += size(leftIndex);
  }

  if (isAvail(rightIndex)) {
    remove(rightIndex);
    sz += size(rightIndex);
  }

  makeAvail(idx, sz);
}

/*****************/
/* C++ operators */
/*****************/

void *operator new(std::size_t size) {
  void *p = malloc(size);
  if (p == 0)
    KPANIC("?", "out of memory");
  return p;
}

void operator delete(void *p) noexcept { return free(p); }

void operator delete(void *p, std::size_t) noexcept { return free(p); }

void *operator new[](std::size_t size) {
  void *p = malloc(size);
  if (p == 0)
    KPANIC("?", "out of memory");
  return p;
}

void operator delete[](void *p) noexcept { return free(p); }

void operator delete[](void *p, std::size_t) noexcept { return free(p); }
