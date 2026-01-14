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

#include "thread.h"

#include "atomic.h"
#include "debug.h"
#include "heap.h"
#include "machine.h"
#include "per_core.h"
#include "print.h"
#include "queue.h"
#include "tls.h"

// Needs to be outside of the namespace for convient access from assembly
thread_local uint64_t *stack;
thread_local Fun *thread_fun;
thread_local uint64_t saved_rbp;
thread_local uint64_t saved_rbx;
thread_local uint64_t saved_r12;
thread_local uint64_t saved_r13;
thread_local uint64_t saved_r14;
thread_local uint64_t saved_r15;
thread_local uint64_t saved_rsp;
thread_local uint64_t saved_rflags;

namespace impl {

Queue<TCB, SpinLock> ready_queue{};

Atomic<int64_t> n_active{0};
Atomic<bool> bootstrapping{true};

bool block_impl(bool must, void (*func)(void *), void *arg) {
  using namespace impl;
  auto next = ready_queue.remove();
  if (next == nullptr) {
    if (must) {
      ASSERT(rdfsbase() != PerCore::get()->idle_thread);
      next = (TCB *)(PerCore::get()->idle_thread);
    } else {
      return false;
    }
  }
  context_switch(next->fs(), func, arg);
  return true;
}

void thread_entry() {
  if (thread_fun != nullptr) {
    (*thread_fun)();
  }

  if (thread_fun != nullptr) {
    delete thread_fun;
    thread_fun = nullptr;
  }

  Thread::stop();
}

[[noreturn]] void event_loop() {
  /* only the idle thread can enter this function */
  ASSERT(rdfsbase() == PerCore::get()->idle_thread);
  while (bootstrapping.get() || (n_active.get() > 0)) {
    ASSERT(rdfsbase() == PerCore::get()->idle_thread);
    if (!impl::block(false, [] {})) {
      pause();
    }
  }
  if (PerCore::id() == 0) {
    SAY("checking leaks\n");
    impl::check_leaks();
    shutdown();
  } else {
    while (true) {
      asm volatile("hlt");
    }
  }
}

} // namespace impl

void Thread::make(Fun *fun) {

  using namespace impl;

  auto *tls = TLS::create(false);
  n_active.fetch_add(1);
  bootstrapping.set(false);

  auto the_stack = new uint64_t[1024];
  ASSERT(((uintptr_t)the_stack & 0xf) == 0);
  ASSERT(((uintptr_t(&the_stack[1023]) + 8) & 0xf) == 0);
  the_stack[1023] = (uint64_t)thread_entry;
  tls->var(thread_fun) = fun;
  tls->var(stack) = the_stack;
  tls->var(saved_rsp) = uint64_t(&the_stack[1023]);
  tls->var(saved_rflags) = 0;
  ready_queue.add(tls->tcb());
}

void Thread::bootstrap() {
  auto tls = TLS::create(true);
  wrfsbase(tls->fs());
}

void Thread::yield() {
  using namespace impl;
  auto me = TCB::current();
  block(false, [me] { ready_queue.add(me); });
}

[[noreturn]] void Thread::stop() {
  using namespace impl;
  if (rdfsbase() == PerCore::get()->idle_thread) {
    event_loop();
  } else {
    ASSERT(n_active.get() > 0);
    block(true, [tcb = TCB::current()] {
      auto tls = tcb->tls();
      auto st = tls->var(stack);
      ASSERT(st != nullptr);
      delete[] st;
      delete[] (char *)tls;
      n_active.fetch_add(-1);
    });
    KPANIC("?\n", "unreachable");
  }
}
