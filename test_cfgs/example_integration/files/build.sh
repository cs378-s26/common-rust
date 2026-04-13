#!/bin/sh
# Google Gemini Generated This
set -e
clang --target=aarch64-none-elf -ffreestanding -fno-pic -fno-stack-protector -c init.c -o init.o
ld.lld -Ttext 0x0 --entry=_start --static --no-dynamic-linker --image-base=0 init.o -o init.elf
llvm-objcopy -O binary init.elf init
