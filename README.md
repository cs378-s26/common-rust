# CS 378 - Multicore Operating Systems - Rust Kernel

## Running

A utility program to launch the kernel is included in this template. You can use the following commands:

```sh
cargo buildtool image # build kernel image
cargo buildtool qemu # runs qemu
cargo buildtool gdb # runs gdb, and attaches to qemu
cargo buildtool clean # cleans the buildtool cache
cargo buildtool help # help message
cargo buildtool test # runs all tests. QEMU flags specified in run_qemu_{arch_string}.sh
cargo buildtool qemu-test {path to .json} [--stdout] # runs the test specified by the .json file.
```

The current buildtool supports both aarch64 & x86-64, though some qemu args may be funky depending on your system.

Use the `-k` flag to enable KVM, which is fairly close to real hardware as far as the processor is concerned. When running `gdb`, you **must**
pass in `-k` if and only if the `qemu` instance was started with `-k`. Use `--help` for more options. You can configure the number of cores and
amount of memory.

**Note:** Your QEMU version **must** be new enough for compatibility with `ovmf.fd`, ideally at least 8.2 (the latest possible with Ubuntu 24.04). 

## Testing

QEMU command is specified in `run_qemu_{arch_string}.sh`.

## TODO
### Testing
- [ ] Support integration tests. Each test should define a qemu run script for each arch & expected serial output files.
### Misc
- [ ] fix kernel symbol module generation

## Further documentation

For more information on the inner working of kernel subcomponents, see the `docs/` directory, which contains markdown files on various topics. The docs are not comprehensive, but they should give you a good starting point for understanding the kernel's design and implementation.
