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

// Ideally, we don't use ../ but works for right now
#include "../../src/debug.h"
#include "../../src/machine.h"
#include "../../src/print.h"

// isa-debug-exit is a QEMU virtual device we can use to automate testing of our kernel

namespace Test {

// Exit codes for isa-debug-exit
enum ExitCode : uint8_t {
  PASS = 0x00, // QEMU exits with code 1
  FAIL = 0x01, // QEMU exits with code 3
};

// Write to isa-debug-exit port to terminate QEMU
[[noreturn]] inline void exit(ExitCode code) {
  outb(0xf4, static_cast<uint8_t>(code));
  ASSERT(!"QEMU should have exited");
}

[[noreturn]] inline void pass() {
  SAY("TEST PASSED\n");
  exit(PASS);
}

[[noreturn]] inline void fail(const char *msg) {
  SAY("TEST FAILED: ?\n", msg);
  exit(FAIL);
}

[[noreturn]] inline void fail_at(const char *file, int line, const char *cond) {
  KPRINT("*** TEST FAILED: ? at ?:?\n", cond, file, Dec(line));
  exit(FAIL);
}

} // namespace Test

// Test assertion macro
#define TEST_ASSERT(cond)                                                                          \
  do {                                                                                             \
    if (!(cond)) {                                                                                 \
      Test::fail_at(__FILE__, __LINE__, #cond);                                                    \
    }                                                                                              \
  } while (0)

// Test assertion macro (custom message)
#define TEST_ASSERT_MSG(cond, msg)                                                                 \
  do {                                                                                             \
    if (!(cond)) {                                                                                 \
      Test::fail(msg);                                                                             \
    }                                                                                              \
  } while (0)

// Mark test as passed and exit
#define TEST_PASS() Test::pass()

// Mark test as failed with message and exit
#define TEST_FAIL(msg) Test::fail(msg)
