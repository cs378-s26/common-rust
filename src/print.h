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

#include "atomic.h"
#include "debug.h"
#include <cstdint>
#include <type_traits>

constexpr char const *const hex_digits = "0123456789abcdef";

template <typename T> class Dec {
  const T value;

public:
  Dec(T const v) : value(v) {
    static_assert(std::is_integral_v<T>, "Not an integer");
  }

  template <typename U> friend void do_print(Dec<U> const d);
};

template <typename T> void do_print(Dec<T> const d) {
  if constexpr (std::is_signed_v<T>) {
    if (d.value < 0) {
      putch('-');
      do_print(Dec(std::make_unsigned_t<T>(-d.value)));
    } else {
      do_print(Dec(std::make_unsigned_t<T>(d.value)));
    }
  } else if (d.value == 0) {
    putch('0');
  } else {
    auto r = d.value % 10;
    auto q = d.value / 10;
    if (q > 0) {
      do_print(Dec(q));
    }
    putch('0' + r);
  }
}

inline void do_print(bool const v) {
  if (v) {
    puts("true");
  } else {
    puts("false");
  }
}

inline void do_print(uint8_t const v) {
  putch(hex_digits[v >> 4]);
  putch(hex_digits[v & 0x0f]);
}

inline void do_print(uint16_t const v) {
  do_print(uint8_t(v >> 8));
  do_print(uint8_t(v & 0xff));
}

inline void do_print(uint32_t const v) {
  do_print(uint16_t(v >> 16));
  do_print(uint16_t(v & 0xffff));
}

inline void do_print(uint64_t const v) {
  do_print(uint32_t(v >> 32));
  do_print(uint32_t(v & 0xffffffff));
}

inline void do_print(int8_t const v) { do_print(uint8_t(v)); }

inline void do_print(int16_t const v) { do_print(uint16_t(v)); }

inline void do_print(int32_t const v) { do_print(uint32_t(v)); }

inline void do_print(int64_t const v) { do_print(uint64_t(v)); }

inline void do_print(char const *str) { puts(str); }

template <typename T> inline void do_print(T const *p) {
  do_print(uintptr_t(p));
}

inline void print(const char *msg) { puts(msg); };

template <typename T, typename... Rest>
void print(const char *msg, T arg, Rest... args) {
  while (true) {
    const char c1 = *msg;
    if (c1 == 0) {
      puts(" ...");
      return;
    }
    msg++;
    if (c1 == '{') {
      const char c2 = *msg;
      if (c2 == '}') {
        msg++;
        do_print(arg);
        print(msg, args...);
        return;
      }
    }
    putch(c1);
  }
}

template <std::size_t Start, std::size_t N> struct StringLiteral {

  char raw_chars[N];

  constexpr StringLiteral(char const (&s)[N]) {
    static_assert(Start < N, "Start exceeds string size");
    for (std::size_t i = 0; i < N; i++) {
      raw_chars[i] = s[i];
    }
  }

  std::size_t size() const { return N - Start; }
  std::size_t start() const { return Start; }

  consteval char const *operator()() const { return &raw_chars[Start]; }

  template <std::size_t M> consteval StringLiteral<M + Start, N> skip() const {
    return StringLiteral<M + Start, N>(raw_chars);
  }
};

template <std::size_t N>
StringLiteral(const char (&)[N]) -> StringLiteral<0, N>;

template <StringLiteral str> void xprint() {
  puts(str());
  // print("{}\n", str());
  // print("{}\n", str.size());
}

template <StringLiteral str, typename T, typename... Ts>
[[gnu::noinline]]
void xprint(T arg, Ts... args) {
  constexpr char c = *str();
  static_assert(c != 0, "End of string reached");
  if constexpr (c == '?') {
    do_print(arg);
    xprint<str.template skip<1>(), Ts...>(args...);
  } else {
    putch(c);
    xprint<str.template skip<1>(), T, Ts...>(arg, args...);
  }
}

namespace impl {
extern SpinLock say_lock;
}

template <StringLiteral str, typename T, typename... Ts>
[[gnu::noinline]] void kprint(T arg, Ts... args) {
  LockGuard lg{impl::say_lock};
  xprint<str>(arg, args...);
}

template <StringLiteral str> [[gnu::noinline]] void kprint() {
  LockGuard lg{impl::say_lock};
  xprint<str>();
}

#define KPRINT(str, ...) kprint<str>(__VA_ARGS__)
#define SAY(str, ...) KPRINT("*** " str, __VA_ARGS__)

#define KDEBUG(str, ...)                                                       \
  kprint<"[?:?] " str>(__FILE__, Dec(__LINE__), __VA_ARGS__)

#define KPANIC(msg, ...)                                                       \
  do {                                                                         \
    xprint<"[?:?] " msg>(__FILE__, Dec(__LINE__), __VA_ARGS__);                \
    shutdown();                                                                \
  } while (0)
