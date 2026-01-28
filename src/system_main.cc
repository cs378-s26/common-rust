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

#include "system_main.h"

#include <cstdint>

#include "core_main.h"
#include "debug.h"
#include "heap.h"
#include "limine++.h"
#include "machine.h"
#include "physmem.h"
#include "print.h"
#include "x86_64.h"

using namespace limine;

extern void (*_init_array_start)();
extern void (*_init_array_end)();

char the_heap[6 * 1024 * 1024];

[[gnu::section(
    ".limine_requests")]] constexpr volatile BaseRevision base_revision{};
[[gnu::section(".limine_requests")]] constexpr volatile MultiProcessor mp{};

//[[gnu::section(".limine_requests")]] constexpr volatile HHDM limine_hhdm{};

uint64_t Sys::core_count = 0;

extern "C" [[noreturn]] void system_main(void) {

  using namespace Sys;

  // Make sure we were loaded by a limine-compliant bootloader that supports the
  // base revision we require
  ASSERT(base_revision.supported());

  ///////////////////////////////////////////////////////////
  // Check supported CPU features and enable required ones //
  ///////////////////////////////////////////////////////////

  Features features{};
  features.dump();
  ASSERT(features.hasSSE3);
  enable_sse();
  ASSERT(features.hasFSGSBASE);
  CR4().setFSGSBASE();
  CR0().dump();
  CR4().dump();

  /////////////////
  // Frequencies //
  /////////////////

  ///////////////////////////
  // Initialize core count //
  ///////////////////////////

  ASSERT(mp.response);

  core_count = mp.response->cpu_count;

  ////////////////////////////////
  // Initialize the kernel heap //
  ////////////////////////////////

  heap::init(uintptr_t(the_heap), sizeof(the_heap));

  ///////////////////////////////////////////////////////////////////////////
  // Run global initializers -- has to be done after initializing the heap //
  ///////////////////////////////////////////////////////////////////////////

  KPRINT("_init_array_start:? _init_array_end:?\n", &_init_array_start,
         &_init_array_end);

  std::size_t count = &_init_array_end - &_init_array_start;
  for (std::size_t i = 0; i < count; i++) {
    (*((&_init_array_start)[i]))();
  }

  /////////////////////////
  // Wake up other cores //
  /////////////////////////

  KPRINT("has ? cores, flags=?, bsp_lapic_id=?\n", Dec(core_count),
         mp.response->flags, mp.response->bsp_lapic_id);

  impl::start_barrier = leak(new SpinBarrier(core_count), true);

  for (uint64_t i = 0; i < core_count; i++) {
    auto info = mp.response->cpus[i];
    if (mp.response->bsp_lapic_id != info->lapic_id) {
      info->extra_argument = 0;
      info->goto_address = impl::core_main;
    }
  }

  impl::core_main(mp.response->cpus[mp.response->bsp_lapic_id]);
}
