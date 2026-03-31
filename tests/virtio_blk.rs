#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_common::test_runner)]

kernel_common::integration_test!({
    use kernel_common::arch::{Arch, ArchTrait};
    use kernel_common::devices::device_discovery::BLOCK_DEVICES;
    use kernel_common::print::kprintln;
    use kernel_common::sync::MutexLike;

    let mut virtio_blk_device = {
        let mut block_devices = BLOCK_DEVICES.lock();
        let idx = block_devices
            .iter()
            .position(|device| device.name() == "virtio_blk")
            .expect("Failed to find virtio block device for testing");
        block_devices.swap_remove(idx)
    }; // lock drops here
    kprintln!("Found virtio block device");
    let block_size = virtio_blk_device.block_size();
    let block_count = virtio_blk_device.block_count();

    assert_eq!(
        block_size,
        Arch::PAGE_SIZE,
        "virtio block device reported an unexpected block size"
    );

    // We use a few blocks near the end of the disk for testing
    assert!(
        block_count >= 4,
        "Need at least 4 blocks to run block device tests"
    );
    let test_blocks = [block_count - 3, block_count - 2, block_count - 1];

    // Sanity check: make sure a plain read works before doing any writes.
    {
        let mut buf = [0u8; Arch::PAGE_SIZE];
        virtio_blk_device
            .read_block(test_blocks[0], &mut buf)
            .expect("Failed to read test block");
        kprintln!("Single-block read sanity check passed");
    }

    // Test 1:
    // Test multi-block read and write. We read 3 blocks, save their original contents,
    // write new data to them, read back and verify, then restore the original contents.
    {
        let [b0, b1, b2] = test_blocks;

        let mut original0 = [0u8; Arch::PAGE_SIZE];
        let mut original1 = [0u8; Arch::PAGE_SIZE];
        let mut original2 = [0u8; Arch::PAGE_SIZE];

        virtio_blk_device
            .read_block(b0, &mut original0)
            .expect("Failed to read original block 0 for multi-block test");
        virtio_blk_device
            .read_block(b1, &mut original1)
            .expect("Failed to read original block 1 for multi-block test");
        virtio_blk_device
            .read_block(b2, &mut original2)
            .expect("Failed to read original block 2 for multi-block test");

        let mut write0 = [0u8; Arch::PAGE_SIZE];
        let mut write1 = [0u8; Arch::PAGE_SIZE];
        let mut write2 = [0u8; Arch::PAGE_SIZE];

        // fill the write buffers with some test data to match later
        for i in 0..Arch::PAGE_SIZE {
            write0[i] = (i & 0xff) as u8;
            write1[i] = ((i * 3 + 7) & 0xff) as u8;
            write2[i] = 0xA5 ^ ((i * 5) as u8);
        }

        let write_blocks: [&[u8]; 3] = [&write0, &write1, &write2];
        virtio_blk_device
            .write_blocks(&[b0, b1, b2], &write_blocks)
            .expect("Failed to write blocks in multi-block test");

        let mut read0 = [0u8; Arch::PAGE_SIZE];
        let mut read1 = [0u8; Arch::PAGE_SIZE];
        let mut read2 = [0u8; Arch::PAGE_SIZE];
        let mut read_blocks: [&mut [u8]; 3] = [&mut read0, &mut read1, &mut read2];

        virtio_blk_device
            .read_blocks(&[b0, b1, b2], &mut read_blocks)
            .expect("Failed to read blocks in multi-block test");

        assert_eq!(read0, write0, "Multi-block test mismatch on first block");
        assert_eq!(read1, write1, "Multi-block test mismatch on second block");
        assert_eq!(read2, write2, "Multi-block test mismatch on third block");

        let restore_blocks: [&[u8]; 3] = [&original0, &original1, &original2];
        virtio_blk_device
            .write_blocks(&[b0, b1, b2], &restore_blocks)
            .expect("Failed to restore original contents after multi-block test");
        virtio_blk_device
            .write_block(b0, &original0)
            .expect("Failed to restore original contents for block 0");
        virtio_blk_device
            .write_block(b1, &original1)
            .expect("Failed to restore original contents for block 1");
        virtio_blk_device
            .write_block(b2, &original2)
            .expect("Failed to restore original contents for block 2");
        kprintln!("Multi-block read/write test passed");
    }
});
