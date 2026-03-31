#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_common::test_runner)]

kernel_common::integration_test!({
    extern crate alloc;
    use alloc::vec;
    use kernel_common::devices::device_discovery::BLOCK_DEVICES;
    use kernel_common::print::kprintln;
    use kernel_common::sync::MutexLike;

    // helper to fill a buffer with a deterministic pattern for testing
    let fill_pattern = |buf: &mut [u8], mul: usize, add: usize| {
        for (i, byte) in buf.iter_mut().enumerate() {
            *byte = ((i * mul + add) & 0xff) as u8;
        }
    };

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

    // We use a few blocks near the end of the disk for testing
    assert!(
        block_count >= 4,
        "Need at least 4 blocks to run block device tests"
    );
    let test_blocks = [block_count - 3, block_count - 2, block_count - 1];
    // Keep block-sized scratch buffers on the heap. Integration tests run on small kernel
    // thread stacks, so page-sized stack arrays add up quickly and can corrupt unrelated state.

    // Test 1:
    // Test multi-block read and write. We read 3 blocks, save their original contents,
    // write new data to them, read back and verify, then restore the original contents.
    {
        let [b0, b1, b2] = test_blocks;
        let block_idxs = [b0, b1, b2];

        let mut original0 = vec![0u8; block_size];
        let mut original1 = vec![0u8; block_size];
        let mut original2 = vec![0u8; block_size];
        let mut original_blocks: [&mut [u8]; 3] = [
            original0.as_mut_slice(),
            original1.as_mut_slice(),
            original2.as_mut_slice(),
        ];

        virtio_blk_device
            .read_blocks(&block_idxs, &mut original_blocks)
            .expect("Failed to read original blocks for multi-block test");

        let mut write0 = vec![0u8; block_size];
        let mut write1 = vec![0u8; block_size];
        let mut write2 = vec![0u8; block_size];
        fill_pattern(&mut write0, 1, 0x00);
        fill_pattern(&mut write1, 3, 0x07);
        fill_pattern(&mut write2, 5, 0xA5);

        let write_blocks: [&[u8]; 3] = [write0.as_slice(), write1.as_slice(), write2.as_slice()];
        virtio_blk_device
            .write_blocks(&block_idxs, &write_blocks)
            .expect("Failed to write blocks in multi-block test");

        let mut read0 = vec![0u8; block_size];
        let mut read1 = vec![0u8; block_size];
        let mut read2 = vec![0u8; block_size];
        let mut read_blocks: [&mut [u8]; 3] = [
            read0.as_mut_slice(),
            read1.as_mut_slice(),
            read2.as_mut_slice(),
        ];

        virtio_blk_device
            .read_blocks(&block_idxs, &mut read_blocks)
            .expect("Failed to read blocks in multi-block test");

        assert_eq!(read0, write0, "Multi-block test mismatch on first block");
        assert_eq!(read1, write1, "Multi-block test mismatch on second block");
        assert_eq!(read2, write2, "Multi-block test mismatch on third block");

        let restore_blocks: [&[u8]; 3] = [
            original0.as_slice(),
            original1.as_slice(),
            original2.as_slice(),
        ];
        virtio_blk_device
            .write_blocks(&block_idxs, &restore_blocks)
            .expect("Failed to restore original contents after multi-block test");
        kprintln!("Multi-block read/write test passed");
    }

    // Test 2:
    // Exercise BlockDevice::read on a byte range that stays entirely within one block.
    // This should use the helper's partial-block path without touching neighboring blocks.
    {
        let [b0, b1, b2] = test_blocks;
        let block_idxs = [b0, b1, b2];

        let mut original0 = vec![0u8; block_size];
        let mut original1 = vec![0u8; block_size];
        let mut original2 = vec![0u8; block_size];
        let mut original_blocks: [&mut [u8]; 3] = [
            original0.as_mut_slice(),
            original1.as_mut_slice(),
            original2.as_mut_slice(),
        ];

        virtio_blk_device
            .read_blocks(&block_idxs, &mut original_blocks)
            .expect("Failed to read original blocks for single-block read test");

        let mut block0 = vec![0u8; block_size];
        let mut block1 = vec![0u8; block_size];
        let mut block2 = vec![0u8; block_size];
        fill_pattern(&mut block0, 5, 0x11);
        fill_pattern(&mut block1, 7, 0x33);
        fill_pattern(&mut block2, 9, 0x55);

        let setup_blocks: [&[u8]; 3] = [block0.as_slice(), block1.as_slice(), block2.as_slice()];
        virtio_blk_device
            .write_blocks(&block_idxs, &setup_blocks)
            .expect("Failed to program blocks for single-block read test");

        let start_in_block = 127usize;
        let read_len = block_size / 2 - 19;
        let mut read_buf = vec![0u8; read_len];
        let bytes_read = virtio_blk_device
            .read(b1 * block_size + start_in_block, &mut read_buf)
            .expect("BlockDevice::read failed for single-block read test");

        assert_eq!(
            bytes_read, read_len,
            "BlockDevice::read returned the wrong byte count for a single-block range"
        );
        assert_eq!(
            &read_buf[..],
            &block1[start_in_block..start_in_block + read_len],
            "BlockDevice::read returned the wrong bytes for a single-block range"
        );

        let restore_blocks: [&[u8]; 3] = [
            original0.as_slice(),
            original1.as_slice(),
            original2.as_slice(),
        ];
        virtio_blk_device
            .write_blocks(&block_idxs, &restore_blocks)
            .expect("Failed to restore original contents after single-block read test");
    }

    // Test 3:
    // Exercise BlockDevice::read across three blocks so we hit all helper paths:
    // partial first block, full middle block(s), and partial final block.
    {
        let [b0, b1, b2] = test_blocks;
        let block_idxs = [b0, b1, b2];

        let mut original0 = vec![0u8; block_size];
        let mut original1 = vec![0u8; block_size];
        let mut original2 = vec![0u8; block_size];
        let mut original_blocks: [&mut [u8]; 3] = [
            original0.as_mut_slice(),
            original1.as_mut_slice(),
            original2.as_mut_slice(),
        ];

        virtio_blk_device
            .read_blocks(&block_idxs, &mut original_blocks)
            .expect("Failed to read original blocks for multi-block read test");

        let mut block0 = vec![0u8; block_size];
        let mut block1 = vec![0u8; block_size];
        let mut block2 = vec![0u8; block_size];
        fill_pattern(&mut block0, 3, 0x21);
        fill_pattern(&mut block1, 11, 0x43);
        fill_pattern(&mut block2, 13, 0x65);

        let setup_blocks: [&[u8]; 3] = [block0.as_slice(), block1.as_slice(), block2.as_slice()];
        virtio_blk_device
            .write_blocks(&block_idxs, &setup_blocks)
            .expect("Failed to program blocks for multi-block read test");

        let start_in_first = 211usize;
        let end_in_last = 333usize;
        let read_len = (block_size - start_in_first) + block_size + end_in_last;
        let mut read_buf = vec![0u8; read_len];
        let bytes_read = virtio_blk_device
            .read(b0 * block_size + start_in_first, &mut read_buf)
            .expect("BlockDevice::read failed for multi-block read test");

        let mut expected = vec![0u8; read_len];
        let first_len = block_size - start_in_first;
        expected[..first_len].copy_from_slice(&block0[start_in_first..]);
        expected[first_len..first_len + block_size].copy_from_slice(&block1);
        expected[first_len + block_size..].copy_from_slice(&block2[..end_in_last]);

        assert_eq!(
            bytes_read, read_len,
            "BlockDevice::read returned the wrong byte count for a multi-block range"
        );
        assert_eq!(
            read_buf, expected,
            "BlockDevice::read returned the wrong bytes for a multi-block range"
        );

        let restore_blocks: [&[u8]; 3] = [
            original0.as_slice(),
            original1.as_slice(),
            original2.as_slice(),
        ];
        virtio_blk_device
            .write_blocks(&block_idxs, &restore_blocks)
            .expect("Failed to restore original contents after multi-block read test");
    }

    // Test 4:
    // Exercise BlockDevice::write within a single block. This should patch only the requested
    // byte range and preserve the prefix and suffix around it.
    {
        let [b0, b1, b2] = test_blocks;
        let block_idxs = [b0, b1, b2];

        let mut original0 = vec![0u8; block_size];
        let mut original1 = vec![0u8; block_size];
        let mut original2 = vec![0u8; block_size];
        let mut original_blocks: [&mut [u8]; 3] = [
            original0.as_mut_slice(),
            original1.as_mut_slice(),
            original2.as_mut_slice(),
        ];

        virtio_blk_device
            .read_blocks(&block_idxs, &mut original_blocks)
            .expect("Failed to read original blocks for single-block write test");

        let mut block0 = vec![0u8; block_size];
        let mut block1 = vec![0u8; block_size];
        let mut block2 = vec![0u8; block_size];
        fill_pattern(&mut block0, 15, 0x12);
        fill_pattern(&mut block1, 17, 0x34);
        fill_pattern(&mut block2, 19, 0x56);

        let setup_blocks: [&[u8]; 3] = [block0.as_slice(), block1.as_slice(), block2.as_slice()];
        virtio_blk_device
            .write_blocks(&block_idxs, &setup_blocks)
            .expect("Failed to program blocks for single-block write test");

        let start_in_block = 73usize;
        let write_len = block_size / 3;
        let mut write_buf = vec![0u8; write_len];
        fill_pattern(&mut write_buf, 23, 0x78);
        let bytes_written = virtio_blk_device
            .write(b1 * block_size + start_in_block, &write_buf)
            .expect("BlockDevice::write failed for single-block write test");

        let mut read0 = vec![0u8; block_size];
        let mut read1 = vec![0u8; block_size];
        let mut read2 = vec![0u8; block_size];
        let mut read_blocks: [&mut [u8]; 3] = [
            read0.as_mut_slice(),
            read1.as_mut_slice(),
            read2.as_mut_slice(),
        ];
        virtio_blk_device
            .read_blocks(&block_idxs, &mut read_blocks)
            .expect("Failed to read back blocks after single-block write test");

        assert_eq!(
            bytes_written, write_len,
            "BlockDevice::write returned the wrong byte count for a single-block range"
        );
        assert_eq!(
            read0, block0,
            "Single-block write unexpectedly modified the block before the target block"
        );
        assert_eq!(
            &read1[..start_in_block],
            &block1[..start_in_block],
            "Single-block write corrupted bytes before the target range"
        );
        assert_eq!(
            &read1[start_in_block..start_in_block + write_len],
            &write_buf[..],
            "Single-block write did not store the requested payload"
        );
        assert_eq!(
            &read1[start_in_block + write_len..],
            &block1[start_in_block + write_len..],
            "Single-block write corrupted bytes after the target range"
        );
        assert_eq!(
            read2, block2,
            "Single-block write unexpectedly modified the block after the target block"
        );

        let restore_blocks: [&[u8]; 3] = [
            original0.as_slice(),
            original1.as_slice(),
            original2.as_slice(),
        ];
        virtio_blk_device
            .write_blocks(&block_idxs, &restore_blocks)
            .expect("Failed to restore original contents after single-block write test");
    }

    // Test 5:
    // Exercise BlockDevice::write across three blocks. This verifies that the helper does a
    // read-modify-write on the first and last blocks while replacing any fully covered middle
    // blocks entirely from the caller's payload.
    {
        let [b0, b1, b2] = test_blocks;
        let block_idxs = [b0, b1, b2];

        let mut original0 = vec![0u8; block_size];
        let mut original1 = vec![0u8; block_size];
        let mut original2 = vec![0u8; block_size];
        let mut original_blocks: [&mut [u8]; 3] = [
            original0.as_mut_slice(),
            original1.as_mut_slice(),
            original2.as_mut_slice(),
        ];
        virtio_blk_device
            .read_blocks(&block_idxs, &mut original_blocks)
            .expect("Failed to read original blocks for multi-block write test");

        let mut block0 = vec![0u8; block_size];
        let mut block1 = vec![0u8; block_size];
        let mut block2 = vec![0u8; block_size];
        fill_pattern(&mut block0, 21, 0x10);
        fill_pattern(&mut block1, 25, 0x20);
        fill_pattern(&mut block2, 27, 0x30);

        let setup_blocks: [&[u8]; 3] = [block0.as_slice(), block1.as_slice(), block2.as_slice()];
        virtio_blk_device
            .write_blocks(&block_idxs, &setup_blocks)
            .expect("Failed to program blocks for multi-block write test");

        let start_in_first = 157usize;
        let end_in_last = 289usize;
        let write_len = (block_size - start_in_first) + block_size + end_in_last;
        let mut write_buf = vec![0u8; write_len];
        fill_pattern(&mut write_buf, 29, 0x40);
        let bytes_written = virtio_blk_device
            .write(b0 * block_size + start_in_first, &write_buf)
            .expect("BlockDevice::write failed for multi-block write test");

        let mut expected0 = block0;
        let mut expected1 = block1;
        let mut expected2 = block2;
        let first_len = block_size - start_in_first;
        expected0[start_in_first..].copy_from_slice(&write_buf[..first_len]);
        expected1.copy_from_slice(&write_buf[first_len..first_len + block_size]);
        expected2[..end_in_last].copy_from_slice(&write_buf[first_len + block_size..]);

        let mut read0 = vec![0u8; block_size];
        let mut read1 = vec![0u8; block_size];
        let mut read2 = vec![0u8; block_size];
        let mut read_blocks: [&mut [u8]; 3] = [
            read0.as_mut_slice(),
            read1.as_mut_slice(),
            read2.as_mut_slice(),
        ];
        virtio_blk_device
            .read_blocks(&block_idxs, &mut read_blocks)
            .expect("Failed to read back blocks after multi-block write test");

        assert_eq!(
            bytes_written, write_len,
            "BlockDevice::write returned the wrong byte count for a multi-block range"
        );
        assert_eq!(
            read0, expected0,
            "Multi-block write produced the wrong first block contents"
        );
        assert_eq!(
            read1, expected1,
            "Multi-block write produced the wrong middle block contents"
        );
        assert_eq!(
            read2, expected2,
            "Multi-block write produced the wrong final block contents"
        );

        let restore_blocks: [&[u8]; 3] = [
            original0.as_slice(),
            original1.as_slice(),
            original2.as_slice(),
        ];
        virtio_blk_device
            .write_blocks(&block_idxs, &restore_blocks)
            .expect("Failed to restore original contents after multi-block write test");
    }

    // Test 6:
    // Zero-length byte reads and writes should succeed without touching the device.
    {
        let b0 = test_blocks[0];
        let mut empty_read: [u8; 0] = [];
        let empty_write: [u8; 0] = [];

        assert_eq!(
            virtio_blk_device
                .read(b0 * block_size, &mut empty_read)
                .expect("Zero-length BlockDevice::read failed"),
            0,
            "Zero-length BlockDevice::read should report zero bytes read"
        );
        assert_eq!(
            virtio_blk_device
                .write(b0 * block_size, &empty_write)
                .expect("Zero-length BlockDevice::write failed"),
            0,
            "Zero-length BlockDevice::write should report zero bytes written"
        );
    }

    kprintln!("BlockDevice::read/write helper tests passed");
});
