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

    let mut buf1 = [0u8; 4096];
    let mut buf2 = [0u8; 4096];
    let mut buf3 = [0u8; 4096];
    let block_idxs = [0, 1, 2];
    buf[0] = 0xAB;
    virtio_blk_device.write_blocks(&[0], &[&buf[..]]).expect("Failed to write block");
    let mut read_buf = [0u8; 4096];
    kprintln!("Reading back the block we just wrote...");
    virtio_blk_device.read_blocks(&[0], &mut [&mut read_buf[..]]).expect("Failed to read block");
    kprintln!("Read data: {:02X}", read_buf[0]);


    
});
