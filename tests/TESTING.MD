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

# Testing Infrastructure

This document describes the testing infrastructure for the common kernel code, including how to run existing tests and how to create new ones.

## Overview

The testing infrastructure consists of two complementary test suites:

1. **Unit Tests** - Run on the host machine (no QEMU required)
   - Test individual components in isolation
   - Use Google Test framework
   - Mock hardware operations via `HOST_BUILD` compilation flag

2. **Integration Tests** - Run inside QEMU
   - Test full kernel behavior with real hardware emulation
   - Use `isa-debug-exit` QEMU device for automated pass/fail reporting

## Prerequisites

### For Unit Tests (lab machines should already come with the Google Test Library)

- C++23 support (`g++`)
- Google Test library (`libgtest-dev`)

On Ubuntu/Debian:
```bash
sudo apt install g++ libgtest-dev
```

### For Integration Tests

- All kernel build dependencies
- QEMU (`qemu-system-x86_64`)


## Directory Structure

```
tests/
├── TESTING.MD              # This documentation
├── unit/                   # Unit tests
│   ├── test_main.cc        # Google Test entry point
│   ├── test_<blank>.cc     # Unit tests for component <blank>
└── integration/            # Integration tests (run in QEMU)
    ├── harness.h           # Test harness with isa-debug-exit support
    ├── test_<blank>.cc     # Integration tests for component <blank>
```

## Running Tests

### Run All Tests

```bash
make test
```

This runs both unit tests and integration tests.

### Run Unit Tests Only

```bash
make unit-test
```

Output shows Google Test results

### Run Integration Tests Only

```bash
make integration-test
```

Each test runs in a separate QEMU instance:

### Run a Specific Integration Test (see INTEGRATION_TESTS in makefile to add integration test names>)

```bash
make run-test-<name>
```

### Clean Test Artifacts

```bash
make clean-tests
```

## Creating Unit Tests

Unit tests run on the host machine using Google Test. They test kernel components that don't require actual hardware.
https://google.github.io/googletest/

### Step 1: Create a Test File 

Create a new file in `tests/unit/` named `test_<component>.cc`:

```cpp
#include <gtest/gtest.h>

// Include the kernel header you're testing
// The HOST_BUILD flag is automatically set during compilation
#include "../../src/your_component.h"
```

### Step 2: Write Test Cases (see example test cases or read Google Test Suite documentation)
### Step 3: Build and Run

### Common Google Test Assertions

| Assertion | Description |
|-----------|-------------|
| `EXPECT_EQ(a, b)` | `a == b` (non-fatal) |
| `ASSERT_EQ(a, b)` | `a == b` (fatal, stops test) |
| `EXPECT_NE(a, b)` | `a != b` |
| `EXPECT_TRUE(cond)` | `cond` is true |
| `EXPECT_FALSE(cond)` | `cond` is false |
| `EXPECT_LT(a, b)` | `a < b` |
| `EXPECT_LE(a, b)` | `a <= b` |
| `EXPECT_GT(a, b)` | `a > b` |
| `EXPECT_GE(a, b)` | `a >= b` |

### HOST_BUILD Mocking

When compiled with `-DHOST_BUILD`, the kernel headers provide mock implementations for hardware operations. This is handled automatically in `src/machine.h`:

If your component uses hardware operations not yet mocked, add mock implementations to the `#ifdef HOST_BUILD` section in `src/machine.h`.

## Creating Integration Tests

### Step 1: Create a Test File

Create a new file in `tests/integration/` named `test_<name>.cc`:

```cpp
#include "harness.h"
// Include other kernel headers as needed

// This function replaces kernel_main for the test
void kernel_main() {
    // Your test code here
    // ...
    TEST_PASS();  // Mark test as passed
}
```

### Step 2: Register the Test
Add your test name to the `INTEGRATION_TESTS` variable in `Makefile`:

```makefile
INTEGRATION_TESTS := your_test_name_here
```

### Step 3: Write Test Logic

Use the test harness macros from `harness.h`:

### Step 4: Build and Run

```bash
make run-test-your_new_test
```

Or run all integration tests:

```bash
make integration-test
```

## CI/CD Integration

### Unit Tests (`.github/workflows/unit-tests.yml`)

- Runs on every push and pull request
- Tests on Ubuntu with GCC

### Integration Tests (`.github/workflows/integration-tests.yml`)

- Runs nightly (2 AM UTC)
- Can be triggered manually
- Tests full kernel in QEMU