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

#include <cstdint>

#include "machine.h"

namespace impl {
extern uintptr_t tls_size;
}

class TLS;

class TCB {
public:
  TCB *self;
  TCB *next;

  TCB() = delete;
  ~TCB() = delete;

  inline TLS *tls() const { return (TLS *)((uintptr_t)this - impl::tls_size); }

  inline uint64_t fs() const { return uintptr_t(this); }

  static inline TCB *current() { return (TCB *)(rdfsbase()); }
};

class TLS {

public:
  TLS() = delete;
  ~TLS() = delete;

  static TLS *create(bool willLeak);
  static void destroy(TLS *tls);

  inline uintptr_t fs() const { return uintptr_t(this) + impl::tls_size; }

  inline TCB *tcb() const { return (TCB *)(fs()); }

  // Get a reference to a volatile variable in a different TLS segment (belongs
  // to a different thread) This is obviously unsafe and should be used with
  // caution. Why is it unsafe? because humans and compilers are allowed to
  // eliminate the possibility of concurrent access to thread local variables.
  //
  // Places where it's safe to use this function:
  //      (1) during thread creation; before adding the thread to the ready
  //          queue
  //
  //      (2) during thread termination; after the thread does its final
  //          context switch
  //
  //      (3) during context switch; to communicate between the
  //          outgoing and incoming threads
  //
  //      (4) if the value is thread-safe (e.g. an atomic variable)
  //
  //      (5) if something like "&var(x)" is used and the pointer is never
  //          dereferenced
  //
  //      ....
  //
  template <typename T> volatile T &var(volatile T &v) const {
    auto offset = rdfsbase() - (uintptr_t)(&v);
    return *(volatile T *)(fs() - offset);
  }
};
