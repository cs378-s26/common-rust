#!/bin/sh
set -x
qemu-system-x86_64 \
    -bios "$1" \
    -drive file="$2",format=raw \
    -no-reboot \
    -d int,cpu_reset \
    -D qemu.log \
    -M smm=off \
    -m 4G \
    -smp 4 \
    -vga std \
    -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
    -serial "$3" \
    # -enable-kvm \
    # -cpu host
