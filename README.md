# Rust Kernel Template

## Extensions

This builds on the common template with some extra features 
- interrupts/preemption
- blocking locks

## Running

A utility program to launch the kernel is included in this template. You can use the following commands:

```sh
cargo buildtool image # build kernel image
cargo buildtool qemu # runs qemu
cargo buildtool gdb # runs gdb, and attaches to qemu
cargo buildtool clean # cleans the buildtool cache
cargo buildtool help # help message
```

The current configuration for `buildtool` only supports `x86_64` on a fairly generic processor.

Use the `-k` flag to enable KVM, which is fairly close to real hardware as far as the processor is concerned. When running `gdb`, you **must**
pass in `-k` if and only if the `qemu` instance was started with `-k`. Use `--help` for more options. You can configure the number of cores and
amount of memory.

**Note:** Your QEMU version **must** be new enough for compatibility with `ovmf.fd`, ideally at least 8.2 (the latest possible with Ubuntu 24.04). 

## TODO

- fix kernel symbol module generation
