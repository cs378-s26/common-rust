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

#include <stdint.h>

#ifdef HOST_BUILD
// Provide mock implementations for unit-testing on host machines (not QEMU)

#include <sched.h>

namespace mock {
inline thread_local uintptr_t fsbase = 0;
inline thread_local uintptr_t gsbase = 0;
} // namespace mock

inline uint64_t get_cr0() { return 0; }
inline uint64_t get_cr2() { return 0; }
inline uint64_t get_cr3() { return 0; }
inline uint64_t get_cr4() { return 0; }
inline uint64_t set_cr4(uint64_t) { return 0; }

inline uint8_t inb(uint16_t) { return 0; }
inline void outb(uint16_t, uint8_t) {}

inline void enable_sse() {}

inline uintptr_t rdfsbase() { return mock::fsbase; }
inline void wrfsbase(uintptr_t val) { mock::fsbase = val; }
inline uintptr_t rdgsbase() { return mock::gsbase; }
inline void wrgsbase(uintptr_t val) { mock::gsbase = val; }

inline void machine_pause() { sched_yield(); }
#define pause() machine_pause()

inline uint64_t rdtsc() { return 0; }

inline void context_switch(uintptr_t, void (*)(void *), void *) {}

#else
// Assembly implementations for QEMU target

extern "C" uint64_t get_cr0();

extern "C" uint64_t get_cr2();

extern "C" uint64_t get_cr3();

extern "C" uint64_t get_cr4();
extern "C" uint64_t set_cr4(uint64_t val);

extern "C" uint8_t inb(uint16_t port);
extern "C" void outb(uint16_t port, uint8_t val);

extern "C" void enable_sse();

extern "C" uintptr_t rdfsbase();
extern "C" void wrfsbase(uintptr_t val);
extern "C" uintptr_t rdgsbase();
extern "C" void wrgsbase(uintptr_t val);

extern "C" void pause();

static inline uint64_t rdtsc() {
  uint64_t a = 0;
  uint64_t d = 0;
  __asm__ volatile("rdtsc" : "=a"(a), "=d"(d));
  return a | (d << 32);
}

extern "C" void context_switch(uintptr_t fs, void (*func)(void *), void *arg);

#endif // HOST_BUILD
