OS_NAME := $(shell uname -s 2>/dev/null)

ifeq ($(OS_NAME),Darwin)
    CC = x86_64-elf-gcc
    CXX = x86_64-elf-g++
    LD = x86_64-elf-g++
else
    CC = gcc
    CXX = g++
    LD = g++
endif

AG_OPT ?= -O2
AG_WARN ?= -Wall -Wextra -Werror
AG_ACCEL ?= tcg,thread=multi
AG_SMP ?= 4
AG_CPU ?= max

CLANG_FORMAT ?= clang-format
CLANG_TIDY ?= clang-tidy

CFLAGS = \
    ${AG_OPT} \
    ${AG_WARN} \
    @common.flags \
    -std=gnu2x

CCFLAGS = \
    ${AG_OPT} \
    ${AG_WARN} \
    @common.flags \
    -std=gnu++23

LDFLAGS = \
    @common.flags \
    -nostdlib \
    -static \
    -T script.ld

SRCFILES := $(shell find src -type f 2>/dev/null | sort)
FORMATFILES := $(shell find src tests -type f \
    \( -name '*.c' -o -name '*.cc' -o -name '*.h' -o -name '*.hpp' \) \
    2>/dev/null)
CFILES := $(filter %.c,$(SRCFILES))
CCFILES := $(filter %.cc,$(SRCFILES))
SFILES := $(filter %.S,$(SRCFILES))
OFILES := $(addprefix build/,$(CFILES:.c=.c.o) $(CCFILES:.cc=.cc.o) $(SFILES:.S=.S.o))
GCC_INCLUDES := $(shell $(CXX) -E -Wp,-v -xc++ /dev/null 2>&1 | grep '^ /' | sed 's/^ /-isystem /')

.PHONY: all
all: build/kernel

build/kernel: common.flags Makefile script.ld $(OFILES)
	@echo "[$@] : $?"
	@mkdir -p build
	$(LD) $(LDFLAGS) $(OFILES) -o $@ -lgcc

build/%.c.o: %.c Makefile common.flags
	@echo "[$@] : $?"
	@mkdir -p "$(dir $@)"
	$(CC) $(CFLAGS) $(CPPFLAGS) -c $< -o $@

build/%.cc.o: %.cc Makefile common.flags
	@echo "[$@] : $?"
	@mkdir -p "$(dir $@)"
	$(CXX) $(CCFLAGS) $(CPPFLAGS) -c $< -o $@

# Compilation rules for *.S files.
build/%.S.o: %.S Makefile common.flags
	@echo "[$@] : $?"
	@mkdir -p "$(dir $@)"
	$(CC) $(CFLAGS) $(CPPFLAGS) -c $< -o $@

limine/limine: Makefile limine/*.c limine/*.h limine/Makefile
	@echo "[$@] : $?"
	make -C limine

LIMINE_FILES := limine.conf limine/limine-bios.sys limine/BOOTX64.EFI


build/kernel.img: Makefile limine/limine build/kernel ${LIMINE_FILES}
	# borrowed from https://codeberg.org/Limine/limine-cxx-template/src/branch/trunk/GNUmakefile
	@echo "[$@] : $?"
	rm -f $@
	# zero-filled 8MB raw disk image
	dd if=/dev/zero bs=1M seek=8 count=0 of=$@
	# create a boot partition
	PATH=$$PATH:/usr/sbin:/sbin sgdisk $@ -n 1:2048 -t 1:ef00 -m 1
	# install BIOS boot sector and stage2
	./limine/limine bios-install $@
	# format the rest as a FAT32 file system (Limine requirement)
	mformat -i $@@@1M
	# create required directories (UEFI and Limine specified)
	mmd -i $@@@1M ::/EFI ::/EFI/BOOT ::/boot ::/boot/limine
	# copy the kernel executable
	mcopy -i $@@@1M build/kernel ::/boot
	# copy the Limine required files
	mcopy -i $@@@1M ${LIMINE_FILES} ::/boot/limine



format:
	$(CLANG_FORMAT) -i $(FORMATFILES)

.PHONY: check
check:
	$(CLANG_FORMAT) --dry-run --Werror $(FORMATFILES)
ifneq ($(strip $(CCFILES)),)
	$(CLANG_TIDY) -header-filter=src/.* $(CCFILES) -- --target=x86_64-pc-none-elf $(CCFLAGS) $(CPPFLAGS) $(GCC_INCLUDES)
endif
ifneq ($(strip $(CFILES)),)
	$(CLANG_TIDY) -header-filter=src/.* $(CFILES) -- --target=x86_64-pc-none-elf $(CFLAGS) $(CPPFLAGS) $(GCC_INCLUDES)
endif

run: build/kernel.img
	qemu-system-x86_64 \
        -accel ${AG_ACCEL} \
        -machine q35 \
        -cpu ${AG_CPU},tsc-freq=1000000000 \
        -smp ${AG_SMP} \
        -m 128m \
        -no-reboot \
        -nographic \
        --monitor none \
        -drive file=build/kernel.img,format=raw \
        -device isa-debug-exit,iobase=0xf4,iosize=0x04 ; exit $$(($$? >> 1))

### UNIT TESTS (these run on the host) ###
HOST_CXX = g++
HOST_CXXFLAGS = \
    -std=gnu++23 \
    -O2 \
    -Wall -Wextra -Werror \
    -DHOST_BUILD \
    -I.

GTEST_LIBS = -lgtest -lgtest_main

UNIT_TEST_SRCS := $(wildcard tests/unit/test_*.cc)
UNIT_TEST_OBJS := $(UNIT_TEST_SRCS:tests/unit/%.cc=build/tests/unit/%.o)

.PHONY: unit-test
unit-test: build/unit_tests
	@echo "Running unit tests..."
	./build/unit_tests

build/unit_tests: $(UNIT_TEST_OBJS)
	@mkdir -p build
	$(HOST_CXX) $(HOST_CXXFLAGS) $^ -o $@ $(GTEST_LIBS)

build/tests/unit/%.o: tests/unit/%.cc
	@echo "[$@] : $<"
	@mkdir -p "$(dir $@)"
	$(HOST_CXX) $(HOST_CXXFLAGS) -c $< -o $@

### INTEGRATION TESTS (these run in QEMU) ###
INTEGRATION_TESTS := threading multicore

.PHONY: integration-test
integration-test: $(addprefix run-test-,$(INTEGRATION_TESTS))

.PHONY: run-test-%
run-test-%: build/test_%.img
	@echo "Running integration test: $*"
	@qemu-system-x86_64 \
		-accel $(AG_ACCEL) \
		-machine q35 \
		-cpu $(AG_CPU),tsc-freq=1000000000 \
		-smp $(AG_SMP) \
		-m 128m \
		-no-reboot \
		-nographic \
		--monitor none \
		-drive file=$<,format=raw \
		-device isa-debug-exit,iobase=0xf4,iosize=0x04 ; \
		EXIT_CODE=$$(($$? >> 1)); \
		if [ $$EXIT_CODE -eq 0 ]; then \
			echo "PASS: $*"; \
		else \
			echo "FAIL: $* (exit code $$EXIT_CODE)"; \
			exit 1; \
		fi

# Build integration test image (uses test file instead of kernel_main)
INTEGRATION_OFILES := $(filter-out build/src/kernel_main.cc.o,$(OFILES))

build/test_%.img: build/test_%/kernel limine/limine ${LIMINE_FILES}
	@echo "[$@] : $?"
	rm -f $@
	dd if=/dev/zero bs=1M seek=8 count=0 of=$@ 2>/dev/null
	PATH=$$PATH:/usr/sbin:/sbin sgdisk $@ -n 1:2048 -t 1:ef00 -m 1 >/dev/null 2>&1
	./limine/limine bios-install $@ >/dev/null 2>&1
	mformat -i $@@@1M
	mmd -i $@@@1M ::/EFI ::/EFI/BOOT ::/boot ::/boot/limine
	mcopy -i $@@@1M build/test_$*/kernel ::/boot/kernel
	mcopy -i $@@@1M ${LIMINE_FILES} ::/boot/limine

build/test_%/kernel: $(INTEGRATION_OFILES) build/test_%/test.o common.flags Makefile script.ld
	@echo "[$@] : $?"
	@mkdir -p "$(dir $@)"
	$(LD) $(LDFLAGS) $(INTEGRATION_OFILES) build/test_$*/test.o -o $@ -lgcc

build/test_%/test.o: tests/integration/test_%.cc
	@echo "[$@] : $<"
	@mkdir -p "$(dir $@)"
	$(CXX) $(CCFLAGS) $(CPPFLAGS) -c $< -o $@


### Test Targets ###
.PHONY: test
test: unit-test integration-test

.PHONY: clean-tests
clean-tests:
	rm -rf build/tests build/test_* build/unit_tests

.PHONY: clean
clean: clean-tests
	-rm -rf build 
	-make -C limine clean

-include ${shell find build -name '*.d' 2> /dev/null}

