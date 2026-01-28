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