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

#include "debug.h"
#include "machine.h"
#include "print.h"

void putch(const char ch) {
  while (!(inb(0x3F8 + 5) & 0x20)) {
  }
  outb(0x3F8, (uint8_t)ch);
}

void puts(const char *str) {
  while (*str) {
    putch(*str);
    str++;
  }
}

[[noreturn]]
void shutdown() {
  while (true) {
    outb(0xf4, 0x00);
    asm("hlt");
  }
}

[[noreturn]]
void assert(const char *file, int line, const char *cond) {
  KPRINT("assertion failed: ? ? ?\n", file, Dec(line), cond);
  shutdown();
}
