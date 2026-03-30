#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_common::test_runner)]

kernel_common::integration_test!({
    use kernel_common::print::kprintln;
    use kernel_common::devices::block::FOUND_BLOCK_DEVICES;
    use kernel_common::sync::MutexLike;
    kprintln!("Found {} block devices", FOUND_BLOCK_DEVICES.lock().len());

    let mut block_devices = FOUND_BLOCK_DEVICES.lock();

    let virtio_blk_device = block_devices
        .iter_mut()
        .find(|device| device.name() == "virtio_blk")
        .expect("Failed to find virtio block device for testing");



    
});
