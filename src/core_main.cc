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

#include "core_main.h"

#include <cstdint>

#include "heap.h"
#include "kernel_main.h"
#include "limine.h"
#include "machine.h"
#include "per_core.h"
#include "print.h"
#include "thread.h"
#include "x86_64.h"

SpinBarrier *impl::start_barrier;

[[noreturn]] void impl::core_main(limine_mp_info *info) {
  KPRINT("bootstrapping core ?\n", info->lapic_id);

  // Check supported CPU features and enable required ones
  Features features{};
  ASSERT(features.hasSSE3);
  enable_sse();
  ASSERT(features.hasFSGSBASE);
  CR4().setFSGSBASE();

  Thread::bootstrap();
  wrgsbase(uintptr_t(leak(new PerCore(info->lapic_id, rdfsbase()), true)));
  KPRINT("bootstrapped core ?\n", Dec(PerCore::id()));

  impl::start_barrier->sync();

  if (info->lapic_id == 0) {
    /* run kernel_main in the first real thread */
    Thread::create([] { kernel_main(); });
  }

  impl::event_loop();
}
