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

#include "limine.h"
#include <cstdint>

namespace limine {

constexpr uint64_t REVISION = 4;

struct BaseRevision {
  const uint64_t id[2] = {UINT64_C(0xf9562b2d5c95a6c8), UINT64_C(0x6a7b384944536bdc)};
  uint64_t revision = REVISION;

  consteval BaseRevision() {}

  bool supported() const volatile { return revision == 0; }
};

struct MemMap {
  const uint64_t id[4] = LIMINE_MEMMAP_REQUEST_ID;
  const uint64_t revision = REVISION;
  limine_memmap_response volatile *response = nullptr;

  consteval MemMap() {}
};

struct HHDM {
  const uint64_t id[4] = LIMINE_HHDM_REQUEST_ID;
  const uint64_t revision = REVISION;
  limine_hhdm_response volatile *response = nullptr;

  consteval HHDM() {}
};

struct MultiProcessor {
  const uint64_t id[4] = LIMINE_MP_REQUEST_ID;
  const uint64_t revision = REVISION;
  limine_mp_response volatile *response = nullptr;

  consteval MultiProcessor() {}
};

} // namespace limine