#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_common::test_runner)]

kernel_common::integration_test!({
    extern crate alloc;

    use kernel_common::{
        devices::discovery::BLOCK_DEVICES,
        fs::{
            ext2::Ext2, vfs::VFS
        },
        print::kprintln,
        sync::MutexLike,
    };
    // Sanity check that all the devices we expect are there
    let mut block_devices = BLOCK_DEVICES.lock();
    let ext2 = Ext2::new_from_block_devices(&mut block_devices)
        .expect("ext2 filesystem not found on attached block devices");
    drop(block_devices);

    let _ = VFS.mount(ext2.clone(), &["/"]).unwrap();

    // Reach /dev via VFS mount traversal, then look up the device inode by name.
    let root = VFS.get_root().expect("VFS root not set");
    let dev_root = VFS
        .partial_lookup(&root, &["/", "dev"])
        .expect("mount traversal to /dev failed");
    let _ = dev_root
        .lookup("ps2kbd")
        .expect("/dev/ps2kbd not found");
    kprintln!("found /dev/ps2kbd via VFS");
});
