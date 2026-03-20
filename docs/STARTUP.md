# System Startup

This document describes the steps taken by the system during startup, from the moment the CPU starts executing code to the point where the kernel is fully initialized.

## Bootloader

We use the Limine bootloader, which runs before our kernel and transfers control to our kernel's entry point. Limine also provides us with various information about the system through its request structs, which can be checked for a response before accessing. If you need to use one of these requests, declare it like so:

```rust
#[used]
#[unsafe(link_section = ".limine_requests")]
pub static BOOTLOADER_INFO_REQUEST: BootloaderInfoRequest = BootloaderInfoRequest::new();
```

You can then check for a response like so:

```rust
if let Some(res) = BOOTLOADER_INFO_REQUEST.get_response() {
    kprintln!("bootloader: {} v{}", res.name(), res.version());
}
```

## system_main()

This is our kernel's entry point, just a wrapper around system_init. Currently, one must define this function in every separate compilation instance (unit tests, main, and every integration test separately), with the correct definition of KernelWork for the application. The `#[no_mangle]` attribute is required to ensure that the function name is not mangled by the Rust compiler, allowing it to be linked with and used as the executable entry point.

```rust
#[unsafe(no_mangle)]
unsafe extern "C" fn system_main() -> ! {
    kernel_common::system_init::<KernelWork>();
}
```

## system_init()

This runs on the bootstrap processor (BSP) and is responsible for initializing certain basic features, then starting the other processors.

## core_init()

This runs on every core, including the BSP. It is responsible for initializing core-local data structures, initializing more kernel features (which could be done on a single core, but there's no compelling reason to), starting a thread that does the relevant work for the application, and then handing off to the scheduler.

The `one!` and `all!` macros run a given initialization code on one (arbitrary) core and all cores, respectively, then ensure that all cores have reached the same point before proceeding.

## KernelEntryTrait

This trait is used to define the entry point of the kernel's first thread. The `work()` function is called after the kernel has been fully initialized and is responsible for running the main logic of the kernel. This trait is used to allow for flexibility in defining the main thread's behavior, as different applications and test cases may have different requirements for what the main thread should do.

```rust
#[cfg(test)]
pub struct KernelWork;

#[cfg(test)]
impl KernelWorkTrait for KernelWork {
    fn work() {
        #[cfg(test)]
        test_main();
    }
}
```
