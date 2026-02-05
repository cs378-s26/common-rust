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

#include "tls.h"
#include "debug.h"
#include "heap.h"
#include <cstdint>

// Pre-conditions (enforced by the linker script):
//    - __tls_start__ points to the read-only original copy of the tls area
//    - __tls_end__ points to the end of the read-only original copy of the tls
//    area
//    - __tls_start__ and __tls_end__ are 16 bytes aligned
//    - __tls_start__ <= __tls_end__

// tls start and end markers
//    - defined in the linker script
//    - used to initialize the tls area
//    - read-only

extern uint8_t __tls_start__[];
extern uint8_t __tls_end__[];

std::size_t impl::tls_size = (&__tls_end__[0]) - (&__tls_start__[0]);

TLS *TLS::create(bool willLeak) {
  using namespace impl;
  const auto base = leak(new uint8_t[tls_size + sizeof(TCB)], willLeak);

  ASSERT((uintptr_t(base) & 0xf) == 0);           // guranateed by the heap implementation
  ASSERT((uintptr_t(&__tls_start__) & 0xf) == 0); // guaranteed by the linker script
  ASSERT((uintptr_t(&__tls_end__) & 0xf) == 0);   // guaranteed by the linker script

  TLS *const tls = (TLS *)base;
  TCB *const tcb = tls->tcb();
  tcb->self = tcb;
  tcb->next = nullptr;

  auto ptr = (uint64_t *)(&__tls_start__[0]);

  for (std::size_t i = 0; i < tls_size / sizeof(uint64_t); i++) {
    ((uint64_t *)base)[i] = ptr[i];
  }

  return tls;
}

// void TLS::destroy(TLS *tls) { delete[] (uint8_t *)tls; }
