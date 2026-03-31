#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_common::test_runner)]

kernel_common::integration_test!({
    use kernel_common::devices::device_discovery::BLOCK_DEVICES;
    use kernel_common::print::kprintln;
    use kernel_common::sync::MutexLike;
    use kernel_common::arch::{Arch, ArchTrait};


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

    kprintln!(
        "virtio block device: block_size = {}, block_count = {}",
        block_size,
        block_count
    );

    assert_eq!(
        block_size,
        Arch::PAGE_SIZE,
        "virtio block device reported an unexpected block size"
    );

    // We use a few blocks near the end of the disk for testing
    assert!(block_count >= 4, "Need at least 4 blocks to run block device tests");
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
    // Read one block, overwrite it with a known pattern, read it back, and make
    // sure the contents match exactly. Then restore the original contents.
    {
        let block_idx = test_blocks[0];

        let mut original = [0u8; Arch::PAGE_SIZE];
        let mut write_buf = [0u8; Arch::PAGE_SIZE];
        let mut read_back = [0u8; Arch::PAGE_SIZE];

        virtio_blk_device
            .read_block(block_idx, &mut original)
            .expect("Failed to read original contents for single-block test");

        kprintln!("1");

        for (i, byte) in write_buf.iter_mut().enumerate() {
            *byte = ((i * 37 + 11) & 0xff) as u8;
        }

        virtio_blk_device
            .write_block(block_idx, &write_buf)
            .expect("Failed to write single test block");
        kprintln!("2");

        virtio_blk_device
            .read_block(block_idx, &mut read_back)
            .expect("Failed to read back single test block");
        kprintln!("3");

        assert_eq!(
            read_back,
            write_buf,
            "Single-block read/write test did not round-trip correctly"
        );

        virtio_blk_device
            .write_block(block_idx, &original)
            .expect("Failed to restore original contents after single-block test");

        kprintln!("Single-block read/write test passed");
    }

    // Test 2:
    // Do the same thing across multiple blocks using the block-device batch
    // interface. This checks that the wrapper correctly walks the index/buffer
    // arrays and that each block lands where expected.
    {
        let [b0, b1, b2] = test_blocks;

        let mut original0 = [0u8; Arch::PAGE_SIZE];
        let mut original1 = [0u8; Arch::PAGE_SIZE];
        let mut original2 = [0u8; Arch::PAGE_SIZE];

        kprintln!("21");
        virtio_blk_device
            .read_block(b0, &mut original0)
            .expect("Failed to read original block 0 for multi-block test");
        virtio_blk_device
            .read_block(b1, &mut original1)
            .expect("Failed to read original block 1 for multi-block test");
        virtio_blk_device
            .read_block(b2, &mut original2)
            .expect("Failed to read original block 2 for multi-block test");
        kprintln!("22");

        let mut write0 = [0u8; Arch::PAGE_SIZE];
        let mut write1 = [0u8; Arch::PAGE_SIZE];
        let mut write2 = [0u8; Arch::PAGE_SIZE];

        for i in 0..Arch::PAGE_SIZE {
            write0[i] = (i & 0xff) as u8;
            write1[i] = ((i * 3 + 7) & 0xff) as u8;
            write2[i] = 0xA5 ^ ((i * 5) as u8);
        }

        kprintln!("23");
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

        kprintln!("24");
        let restore_blocks: [&[u8]; 3] = [&original0, &original1, &original2];
        // virtio_blk_device
        //     .write_blocks(&[b0, b1, b2], &restore_blocks)
        //     .expect("Failed to restore original contents after multi-block test");
        virtio_blk_device.write_block(b0, &original0)
            .expect("Failed to restore original contents for block 0");
        kprintln!("24.5");
        virtio_blk_device.write_block(b1, &original1)
            .expect("Failed to restore original contents for block 1");
        kprintln!("24.75");
        virtio_blk_device.write_block(b2, &original2)
            .expect("Failed to restore original contents for block 2");
        kprintln!("25");

        kprintln!("Multi-block read/write test passed");
    }

});
