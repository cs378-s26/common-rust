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
#include "machine.h"

class PerCore {
public:
  PerCore(uint64_t lapic_id, uint64_t idle_thread) : lapic_id(lapic_id), idle_thread(idle_thread) {}
  const uint64_t lapic_id;
  const uint64_t idle_thread;

  static uint64_t core_count;
  static inline uint64_t id() { return PerCore::get()->lapic_id; }
  static inline PerCore *get() { return (PerCore *)(rdgsbase()); }
};