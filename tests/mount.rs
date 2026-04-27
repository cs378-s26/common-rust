#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_common::test_runner)]

kernel_common::integration_test!({
    extern crate alloc;

    use kernel_common::{
        devices::discovery::BLOCK_DEVICES,
        fs::{ext2::Ext2, vfs::VFS},
        sync::MutexLike,
    };
    let mut block_devices = BLOCK_DEVICES.lock();
    let ext2 = Ext2::new_from_block_devices(&mut block_devices)
        .expect("ext2 filesystem not found on attached block devices");
    drop(block_devices);

    let _ = VFS.mount(ext2.clone(), &["/"]).unwrap();
    let _ = VFS.mount(ext2.clone(), &["/", "cat"]).unwrap();
    let root = VFS.get_root().unwrap();
    let mut path = alloc::vec!["/", "cat"];
    let cat = VFS.partial_lookup(&root, &path).unwrap();
    path.push("hello.txt");
    let hello = VFS.partial_lookup(&cat, &path).unwrap();
    let mut buffer = alloc::vec![0u8; 1024];
    hello.read_unaligned(0, &mut buffer).unwrap();
    assert!(buffer[0] == b'e');
});
