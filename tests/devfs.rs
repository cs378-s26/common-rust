#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_common::test_runner)]

kernel_common::integration_test!({
    extern crate alloc;

    use alloc::sync::Arc;

    use kernel_common::{
        fs::{
            dev::allocate_device_inode,
            vfs::{FsError, VFSDevice, VFS},
            ext2::Ext2,
        },
        sync::MutexLike,
        devices::discovery::BLOCK_DEVICES,
        print::kprintln,
    };

    // Trivial device: reads zeros, silently accepts writes.
    struct NullDevice;

    impl VFSDevice for NullDevice {
        fn read_unaligned(&self, _offset: usize, buffer: &mut [u8]) -> Result<usize, FsError> {
            buffer.fill(0);
            Ok(buffer.len())
        }

        fn write_unaligned(&self, _offset: usize, buffer: &[u8]) -> Result<usize, FsError> {
            Ok(buffer.len())
        }
    }

    let mut block_devices = BLOCK_DEVICES.lock();
    let ext2 = Ext2::new_from_block_devices(&mut block_devices)
        .expect("ext2 filesystem not found on attached block devices");
    drop(block_devices);

    let _ = VFS.mount(ext2.clone(), &["/"]).unwrap();

    // Register the device at /dev/null_test.
    allocate_device_inode("null_test", Arc::new(NullDevice))
        .expect("failed to register NullDevice in DevFS");
    kprintln!("registered /dev/null_test");

    // Reach /dev via VFS mount traversal, then look up the device inode by name.
    let root = VFS.get_root().expect("VFS root not set");
    let dev_root = VFS
        .partial_lookup(&root, &["/", "dev"])
        .expect("mount traversal to /dev failed");
    let null_node = dev_root
        .lookup("null_test")
        .expect("/dev/null_test not found");
    kprintln!("found /dev/null_test via VFS");

    // Read: device should fill the buffer with zeros.
    let mut buf = alloc::vec![0xFFu8; 8];
    let n = null_node
        .read_unaligned(0, &mut buf)
        .expect("read_unaligned failed");
    assert_eq!(n, 8, "expected 8 bytes read");
    assert!(buf.iter().all(|&b| b == 0), "expected all-zero bytes from NullDevice");
    kprintln!("read check passed");

    // Write: device should accept the write and report the correct byte count.
    let payload = b"hello devfs";
    let written = null_node
        .write_unaligned(0, payload)
        .expect("write_unaligned failed");
    assert_eq!(written, payload.len(), "unexpected write byte count");
    kprintln!("write check passed");

    kprintln!("devfs integration test passed");
});
