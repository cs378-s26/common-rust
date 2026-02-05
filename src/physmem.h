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

#include <cstdint>

namespace impl {
struct Range;
} // namespace impl

class PA;
class PPN;

class PhysMem {
  impl::Range *free_ranges;
  uint64_t total_memory;

public:
  constexpr static uint64_t LOG_FRAME_SIZE = 12;
  constexpr static uint64_t FRAME_SIZE = 1 << LOG_FRAME_SIZE;

  PhysMem();
  PPN alloc();
  void free(PPN ppn);
};

class PA {
public:
  uint64_t const pa;
  explicit constexpr inline PA(uint64_t pa) : pa(pa) {}
};

class PPN {
public:
  uint64_t const ppn;
  explicit constexpr inline PPN(uint64_t ppn) : ppn(ppn) {}
  explicit constexpr inline PPN(PA pa) : ppn(pa.pa >> PhysMem::LOG_FRAME_SIZE) {}
  explicit constexpr inline operator PA() const { return PA(ppn << PhysMem::LOG_FRAME_SIZE); }
};

extern PhysMem physMem;
