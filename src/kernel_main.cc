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

#include "kernel_main.h"

#include "atomic.h"
#include "count_down_latch.h"
#include "per_core.h"
#include "print.h"
#include "system_main.h"
#include "thread.h"

constexpr int NUM_THREADS = 1000000;

Atomic<uint64_t> *core_counters = new Atomic<uint64_t>[Sys::core_count]();
Atomic<uint64_t> core_changes{};
CountDownLatch latch{};

class Thing {
public:
  Thing() { latch.up(); }
  Thing(const Thing &) { latch.up(); }
  ~Thing() { latch.down(); }
};

void kernel_main() {
  for (int i = 0; i < NUM_THREADS; i++) {
    Thread::create([t = Thing()] {
      const auto c1 = PerCore::id();
      core_counters[PerCore::id()].fetch_add(1);
      Thread::yield();
      const auto c2 = PerCore::id();
      core_counters[PerCore::id()].fetch_add(1);
      if (c1 != c2) {
        core_changes.fetch_add(1);
      }
    });
    Thread::yield();
  }
  latch.sync();
  SAY("made it\n");

  uint64_t sum = 0;
  for (uint64_t i = 0; i < Sys::core_count; i++) {
    sum += core_counters[i].get();
    if (core_counters[i].get() == 0) {
      SAY("Core ? didn't do any work\n", Dec(i));
    }
  }

  if (sum == NUM_THREADS * 2) {
    SAY("Good sum\n");
  } else {
    SAY("Bad sum ?\n", Dec(sum));
    for (uint64_t i = 0; i < Sys::core_count; i++) {
      SAY("core ? did ?\n", Dec(i), Dec(core_counters[i].get()));
    }
  }

  if (core_changes.get() == 0) {
    SAY("no core changes!\n");
  }

  delete[] core_counters;
#if 0
  uint64_t a = rdtsc();
  while (true) {
    asm volatile("lfence");
    uint64_t b = rdtsc();
    uint64_t delta = b - a;
    if (delta < uint64_t(1000000000)) {
      pause();
      continue;
    }
    SAY("?\n", Dec(delta));
    a = b;
  }
#endif
}
