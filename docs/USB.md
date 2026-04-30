## Overview

A stage-1 xHCI USB host-controller driver under `src/devices/usb/xhci/`. It brings the controller up at boot and enumerates every device plugged into a root port — reading each one's descriptors and addressing it on the bus — and stops there. There are no USB class drivers yet, no API for issuing transfers from the rest of the kernel, no support for USB hubs, and no hot-plug.

The driver hooks into device discovery as a normal PCIe driver: it matches xHCI controllers by their PCIe class code, runs its bringup sequence, and returns `Special` so the discovery layer doesn't try to wrap it in a generic block/char/network handle. Anything else in the kernel that wants to see the discovered controllers reaches for the `CONTROLLERS` global, which holds one entry per controller; each controller exposes its addressed devices through a `devices` field that lists the parsed descriptors and the per-device control-endpoint transfer ring.

## Bringup

`XhciController::bringup` runs in round-2 device discovery (PCIe is only initialized after ACPI parsing).  it walks the controller from "first time the OS has touched it" to "running, with interrupts wired and an enumeration worker spawned":

1. **Take ownership of the controller's hardware.** Turn on the PCIe device's memory-mapped IO and bus-mastering bits, then map its BAR0 register block.
2. **Read the controller's capabilities** — how many devices it can address (slots), how many root ports it has, whether its per-slot context structs are 32 or 64 bytes, where its doorbell and runtime register sets live inside BAR0.
3. **Take the controller away from the BIOS.** On boot, BIOS may still own the xHCI to keep legacy USB keyboards working before an OS is loaded. We claim the "OS-owned" bit, wait for the BIOS-owned bit to clear, and mask off the BIOS's SMI traps so firmware can't keep snooping on USB traffic behind our back.
4. **Halt and reset** the controller — stop it if it was running, then issue a hardware reset so it starts from a known clean state.
5. **Hand the controller the DMA structures it'll read and write.** It needs three: the *Device Context Base Address Array* (DCBAA), which points at a per-device context block for each slot; the *Command Ring*, where the OS posts requests to the HC; and the *Event Ring*, where the HC posts results back. We allocate each as a `DmaRegion` and program their physical addresses into the controller's registers. **The Event Ring's base-address register (ERSTBA) must be programmed last** on the interrupter — the HC starts using the interrupter the moment ERSTBA is written, so the ring's size, dequeue pointer, and interrupt-mask registers all have to be in place first.
6. **Wire up interrupts.** Try MSI-X first; on hardware that only exposes legacy MSI ([see below](#msi-vs-msi-x)), fall back to that. The handler is a small closure that drains the Event Ring and acks the controller.
7. **Start the controller** with interrupts enabled, then spin until it confirms it's actually running (its "halted" status bit clears).
8. **Spawn a worker thread for `enumerate_ports`.** Round-2 discovery runs with CPU IRQs off, but `enumerate_ports` issues commands that complete via interrupt-delivered events — doing this inline would block the BSP forever. Using a normal worker thread (where IRQs are on and blocking is safe) sidesteps it.


## MSI vs MSI-X

xHCI hardware can expose MSI-X, legacy MSI, or both. Bringup prefers MSI-X and falls back to  MSI when it's absent. Without that fallback, controllers that only expose legacy MSI come up cleanly but never deliver events, so every `Promise::get()` deadlocks and enumeration silently hangs. This was the issue I had getting it working on real HW for a while not having MSI support....

## Testing

`tests/usb_xhci.rs` boots a kernel with `qemu-xhci` + `usb-kbd` + `usb-mouse` attached, polls `CONTROLLERS` until enumeration finishes, and asserts that the controller has interfaces matching HID boot keyboard `(0x03, 0x01, 0x01)` and HID boot mouse `(0x03, 0x01, 0x02)`. Run with:

```
cargo buildtool qemu-test test_cfgs/usb_xhci/usb_xhci_x86_64_test.json --stdout
```

For testing without the test runner, `cargo buildtool qemu --usb` boots a normal kernel with the same three QEMU devices attached.

## Current Limits

- No class drivers and no transfer API outside `xhci/` — discovered HID devices sit there but nothing reads keystrokes or pointer events from them.
- No hub support.
- No hot-plug — `PORT_STATUS_CHANGE_EVENT` is dropped.
- Single MSI/MSI-X vector.
- No isochronous and no HS high-bandwidth — `endpoint_context_dwords` sets MaxBurstSize=0 and a one-packet ESIT payload.
- Always `SET_CONFIGURATION` to the first configuration the device reports.
