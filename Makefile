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

CFLAGS = \
    ${AG_OPT} \
    ${AG_WARN} \
    @common.flags \
    -std=gnu23

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
CFILES := $(filter %.c,$(SRCFILES))
CCFILES := $(filter %.cc,$(SRCFILES))
SFILES := $(filter %.S,$(SRCFILES))
OFILES := $(addprefix build/,$(CFILES:.c=.c.o) $(CCFILES:.cc=.cc.o) $(SFILES:.S=.S.o))

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
	clang-format -i src/*.cc src/*.h

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

.PHONY: clean
clean:
	-rm -rf build 
	-make -C limine clean

-include ${shell find build -name '*.d' 2> /dev/null}

