# Devices

This document describes the kernel's device layer: the traits in `src/devices/`, how drivers are discovered, and how device code plugs into other subsystems like boot, virtual memory, printing, DMA, and architecture-specific control paths.

## Overview

The kernel's device model is intentionally small. There is:

- a tiny common base trait, `Device`
- three typed interfaces for character, block, and network devices
- a discovery layer that turns firmware data into initialized drivers
- a few "special" discovery drivers that do not expose a normal device handle, but instead wire hardware directly into some other kernel subsystem

This is not a Unix-style device manager yet. There is no `devfs`, no file-descriptor layer, and no syscall surface that exposes these devices to userspace. Kernel code currently interacts with devices by taking them directly from the global registries in `src/devices/discovery/mod.rs`.

## Core Interfaces

### `Device`

`src/devices/mod.rs` defines the common base trait:

```rust
pub trait Device {
    fn ioctl(&self, request: u64, arg1: u64, arg2: u64) -> u64;
}
```

Right now this is mostly a placeholder for out-of-band control operations. The existing concrete drivers return `0` from `ioctl`, so the meaningful interfaces live in the typed traits below.

### `CharDevice`

`src/devices/char/mod.rs` defines byte-stream I/O:

```rust
pub trait CharDevice: Device {
    fn read(&mut self, buffer: &mut [u8]) -> Result<usize, CharDeviceError>;
    fn write(&self, buffer: &[u8]) -> Result<usize, CharDeviceError>;
}
```

The only current implementation is the PL011 UART driver on AArch64. It is currently output-only in practice: `write()` is implemented, while `read()` returns an error.

### `BlockDevice`

`src/devices/block/mod.rs` is the richest interface. Drivers provide:

- `name()`
- `block_size()` and `block_count()`
- whole-block `read_blocks()` and `write_blocks()`
- `flush()`
- `dma_physical_address_size()`

The trait also provides default `read()` and `write()` helpers for byte-granular access. Those helpers:

- split a request into a partial first block, zero or more full middle blocks, and a partial last block
- batch the fully covered blocks through `read_blocks()` / `write_blocks()`
- use read-modify-write for partial writes

This is the main way the device layer smooths over hardware granularity for higher layers. A caller can ask for byte ranges even though the underlying hardware works in blocks.

### `NetworkDevice`

`src/devices/network/mod.rs` defines packet send/receive operations, but there is no concrete implementation yet. The trait exists as an extension point for future drivers.

## Discovery And Registration

Discovery lives in `src/devices/discovery/`.

### `DeviceDiscovery`

Each discovery driver implements:

```rust
pub trait DeviceDiscovery {
    fn am_i_this(&self, node: DeviceNode) -> Option<Vec<DeviceType>>;
    fn run_at_start(&self) -> bool;
    fn name(&self) -> &'static str;
}
```

`DeviceNode` is the firmware object currently being examined. Today that can be:

- a device-tree node (`DTB`)
- an ACPI MADT entry
- an ACPI MCFG table

`DeviceType` is what discovery returns after a match:

- `Block(Box<dyn BlockDevice + Send + Sync>)`
- `Char(Box<dyn CharDevice + Send + Sync>)`
- `Network(Box<dyn NetworkDevice + Send + Sync>)`
- `Special`

`Special` is important: it means "this hardware was recognized and initialized, but it integrates with the kernel through some path other than the normal device registries."

### Driver Table

`create_drivers()` populates `SYSTEM_DRIVERS` with the discovery drivers compiled into the kernel:

- `UartPl011Discovery`
- `VirtioDiscovery`
- architecture-specific discovery drivers from `Arch::create_arch_specific_drivers()`

On AArch64, that currently adds PSCI discovery and Cortex-A15 GIC discovery. On x86-64 there are no extra discovery drivers yet.

### Firmware Walk

`discover_devices()` collects devices from two sources:

- `parse_acpi()`
- `parse_device_tree()`

ACPI support is currently limited to the pieces needed for device discovery: MADT entries and the MCFG table. Device-tree support scans the DTB through the registered discovery drivers.

For DTB discovery, matching is first-match per node. `parse_device_tree()` iterates DTB nodes outermost; for each node it walks `SYSTEM_DRIVERS` in registration order, appends the first returned `DeviceType` values, and then breaks to the next node. That means driver order matters, and each DTB node is claimed by at most one discovery driver.

### Global Registries

Once discovery returns `DeviceType` values, `discover_devices()` sorts them into global registries:

- `BLOCK_DEVICES`
- `CHAR_DEVICES`
- `NETWORK_DEVICES`

Each registry is an `IntMutex<Vec<Box<dyn ...>>>`. The registry itself is synchronized, but the kernel does not automatically wrap each individual device in a lock. After a caller removes or borrows a device, that caller is responsible for serializing access correctly.

The tests in `tests/virtio_blk.rs` show the current usage model clearly: they lock `BLOCK_DEVICES`, remove the virtio block device from the vector, and then operate on it directly.

## Kernel Integration

### Boot Sequence

There are two rounds of device discovery. The first happens early in `system_init()` in `src/lib.rs`, after:

- heap initialization
- TTY initialization
- physical memory allocator setup
- virtual memory allocator setup

and before:

- SMP bring-up
- per-core initialization
- scheduler handoff

The second happens late in `core_init`, after 

- APIC/GiC initialization
- event handler initilization
- idle thread initialization

Drivers where `run_at_start` is true will run in the first phase, otherwise they run in the second phase.
PCI devices are required to run in the second phase of device discovery. By default, drivers run in the
second phase of initialization, which is what we recommend.

### Printing And Console Output

The device layer already feeds directly into kernel printing.

On x86-64, `arch::init_tty()` installs a 16550 serial backend directly, so serial output does not go through device discovery.

On AArch64, the PL011 UART is found by normal DTB-based discovery. `UartPl011Discovery`:

- matches `arm,pl011`
- maps the UART MMIO page with `PagingOptions::DEVICE_MEMORY`
- creates a `UartPl011Driver`
- hands it to `print::set_serial_backend()`
- returns `DeviceType::Special`

So on AArch64 the "device layer" is what finishes wiring up serial output for `kprint!` and `kprintln!`.

### Virtual Memory And MMIO

Most real devices need MMIO mappings, so device code depends heavily on the virtual-memory layer.

Examples:

- `UartPl011Driver::init()` creates a `VirtualMemoryAllocation` for the UART registers.
- `VirtioDiscovery` temporarily maps the virtio MMIO header so it can identify the device, then installs a permanent shadow mapping for the transport.
- `GicA15Driver::init()` maps the distributor and CPU-interface pages for the interrupt controller.
- `dma::MmioRegion` exists as a reusable helper for drivers that need volatile access to register blocks.

These mappings are created with `PagingOptions::DEVICE_MEMORY` so the CPU treats them as device memory rather than normal cached RAM.

### Physical Memory And DMA

The device layer also leans on the physical-memory allocator.

The current virtio block driver uses the `virtio_drivers` crate, which expects a HAL implementation. `src/devices/block/virtio_blk.rs` provides that HAL by:

- allocating physically contiguous frames with `alloc_frames()`
- using the higher-half direct map (HHDM) for CPU access to those buffers
- deallocating frames with `frame_dealloc()`
- mapping device MMIO through `MmioRegion`

So the device interface itself stays simple, but actual drivers still reach down into `dma.rs`, `physical_memory.rs`, and `virtual_memory.rs` when they need to talk to hardware.

### Interrupts

Devices can register interrupt handlers via `arch::register_irq_handler`. Interrupt handlers
will run in interrupt context, and most return `None` if they cannot handle the interrupt.

### Architecture-Specific Control Paths

Some discovered hardware is not exposed as a normal `BlockDevice` or `CharDevice`, but is still essential to the kernel.

#### PSCI on AArch64

`PSCIDiscovery` parses the PSCI node from the DTB and stores a singleton `PSCI_DEVICE`. Later, `Arch::shutdown()` uses that singleton to power the machine off. This is why PSCI returns `DeviceType::Special` instead of a normal device handle.

#### GIC on AArch64

`GicA15Discovery` maps the GIC distributor and CPU interface, programs the timer interrupt, and stores the resulting virtual base addresses in atomics. The interrupt code in `src/arch/aarch64/gic.rs` then uses those addresses to:

- acknowledge interrupts
- signal end-of-interrupt
- enable timer interrupts on each core

This is a good example of the boundary of the current device interface: there is no generic `handle_irq()` method on `Device`. Interrupt handling still lives in architecture-specific code, and discovery is mostly responsible for making the controller usable.

## Current Boundaries

The current device layer is useful, but deliberately incomplete.

- The concrete implementations are still sparse: PL011 UART, virtio-mmio block, PSCI, and the AArch64 GIC.
- `NetworkDevice` is only a trait today.
- `ioctl()` is a stub in the existing drivers.
- There is no VFS or syscall integration yet, so devices are kernel-internal objects rather than user-visible files.
- Discovery is tightly coupled to firmware parsing, not to hotplug or runtime bus enumeration.

That makes the current model best thought of as an early kernel driver framework: enough structure to discover hardware, map it safely, and expose typed operations to the rest of the kernel, but not yet a full device-management stack.

## PCI-E Interface

### Initialization

Initialization for PCI-E devices is required to run in round 2 of device discovery. On initialization, an instance
of `PcieFunction` will be passed to `i_am_this`. Several helper functions are provided:

`initialize_bars`: maps all memory-mapped BARS. This only initializes memory-mapped BARs, since modern hardware tends
not to use i/o space. To access BAR `i`, access index `i` of the returned array. For 64-bit BARs, access the number
of the lower BAR. For example, if you want to access an entry spanning `BAR2` and `BAR3`, then access index `2` of
the returned array. 