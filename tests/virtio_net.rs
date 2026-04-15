#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_common::test_runner)]

kernel_common::integration_test!({
    extern crate alloc;
    use alloc::vec;
    use kernel_common::devices::device_discovery::NETWORK_DEVICES;
    use kernel_common::print::kprintln;
    use kernel_common::sync::MutexLike;

    let mut virtio_net_device = {
        let mut network_devices = NETWORK_DEVICES.lock();
        let idx = network_devices
            .iter()
            .position(|device| device.name() == "virtio_net")
            .expect("Failed to find virtio net device for testing");
        network_devices.swap_remove(idx)
    }; // lock drops here

    // Test 1: Discovery
    // Verify the NIC was found, initialized, and registered by the device framework.
    // This proves the full path: device tree node -> MMIO map -> transport -> VirtIONetDriver -> NETWORK_DEVICES.
    assert_eq!(virtio_net_device.name(), "virtio_net");
    kprintln!("virtio_net: device discovered and initialized");

    // Test 2: Send
    // Build a minimal valid ethernet frame and send it. This exercises DMA allocation,
    // the share/unshare bounce buffer path, and the virtio TX queue.
    // Frame layout: 6-byte dst MAC (broadcast), 6-byte src MAC (zeroed), 2-byte ethertype, 46-byte payload.
    let mut frame = vec![0u8; 60];
    frame[0..6].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]); // dst: broadcast
    frame[6..12].copy_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00]); // src: zeroed
    frame[12..14].copy_from_slice(&[0x08, 0x00]); // ethertype: IPv4

    virtio_net_device
        .send_packet(&frame)
        .expect("virtio_net: send_packet failed");
    kprintln!("virtio_net: send_packet succeeded");

    // Test 3: Receive
    // Attempt a receive. No peer is sending in the test environment so a ReceiveError is expected
    // and acceptable — the important thing is that the call returns cleanly without panicking.
    let mut recv_buf = vec![0u8; 1536];
    match virtio_net_device.receive_packet(&mut recv_buf) {
        Ok(len) => kprintln!("virtio_net: receive_packet got {} bytes", len),
        Err(_) => kprintln!("virtio_net: receive_packet returned no packet (expected)"),
    }
});
