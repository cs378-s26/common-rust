#!/bin/sh
set -x
qemu-system-aarch64 \
    -machine virt \
    -cpu cortex-a72 \
    -bios "$1" \
    -drive if=none,id=hd0,file="$2",format=raw \
    -device virtio-blk-device,drive=hd0 \
    -no-reboot \
    -d int,cpu_reset \
    -D qemu.log \
    -m 4G \
    -smp 4 \
    -device ramfb \
    -serial "$3"