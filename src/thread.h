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

#include "fun.h"
#include "heap.h"

namespace impl {
extern bool block_impl(bool must, void (*func)(void *), void *arg);
template <typename Work> inline bool block(bool must, Work work) {
  return block_impl(must, caller<Work>, &work);
}
[[noreturn]] void event_loop();
} // namespace impl

namespace Thread {
void make(Fun *fun);
template <typename Work> void create(Work const &work) {
  make(leak(new FunImpl<Work>{work}, false));
}
void bootstrap();
[[noreturn]] void stop();
void yield();
} // namespace Thread
