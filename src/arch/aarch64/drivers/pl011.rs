use kernel_common::print::SERIAL_BACKEND;
use kernel_common::virtual_memory::{VirtualMemoryAllocation, PagingOptions};
use kernel_common::arch::{Arch::get_address_space, SerialCharSink};
pub fn pl011_init(dev: &mut DeviceInfo) {
    // Implementation for initializing PL011 UART driver
    if let Some(mmio_virt) = dev.mmio_virt {
        base = mmio_virt.0;
        size = mmio_virt.1;
        let options = PagingOptions::Present | PagingOptions::Writable;
        // VirtualMemoryAllocation::new(base, size, options).expect("Failed to allocate virtual memory for PL011 UART");
        let space = get_address_space();
        let backing = Some(base);
        let vm = VirtualMemoryAllocation::new(space, size, backing, options);
        core::mem::forget(vm); // Prevent unmapping
        SERIAL_BACKEND.call_once(|| SerialCharSink::open())

    } else {
        panic!("Failed to allocate virtual memory for PL011 UART");
    }

}