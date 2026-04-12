#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_common::test_runner)]

kernel_common::integration_test!({
    extern crate alloc;

    use alloc::sync::Arc;

    use kernel_common::{
        devices::discovery::BLOCK_DEVICES,
        fs::{
            ext2::Ext2,
            vfs::{Filesystem, INodeType, VFS, VNode},
        },
        print::kprintln,
        sync::MutexLike,
    };

    fn read_text(node: &Arc<dyn VNode>, len: usize) -> alloc::vec::Vec<u8> {
        let mut buf = alloc::vec![0u8; len];
        let n = node
            .read_unaligned(0, &mut buf)
            .expect("read_unaligned failed");
        buf.truncate(n);
        buf
    }

    // from the buildtool, we know the disk with virtio-blk on it should be present and have an ext2 fs on it,
    // the directory structure can be seen in test_cfgs/exts/files
    let mut block_devices = BLOCK_DEVICES.lock();
    let ext2 = Ext2::new_from_block_devices(&mut block_devices)
        .expect("ext2 filesystem not found on attached block devices");
    drop(block_devices);

    let root = ext2.get_root().expect("failed to get ext2 root");
    let _fs_id = VFS.mount(ext2.clone());
    VFS.set_root(root.clone()).ok();

    kprintln!("Mounted ext2 from testfs");

    let root = VFS.get_root().expect("root not set");
    kprintln!("vfs root set");

    // 1) Existing root file can be found and read through VFS.
    let hello = root.lookup("hello.txt").expect("missing hello.txt");
    kprintln!("found hello.txt");
    let hello_bytes = read_text(&hello, 32);
    assert_eq!(
        core::str::from_utf8(&hello_bytes).unwrap(),
        "ext2 hello world"
    );

    // 2) Existing nested directory structure can be traversed and read.
    let dir = root.lookup("dir").expect("missing dir");
    kprintln!("found dir");
    let nested = dir.lookup("nested.txt").expect("missing nested.txt");
    kprintln!("found nested.txt");
    let nested_bytes = read_text(&nested, 32);
    assert_eq!(
        core::str::from_utf8(&nested_bytes).unwrap(),
        "nesting works!"
    );

    // 3) VFS inode-key round trip works.
    let hello_key = hello.get_inode_key().expect("hello.txt missing inode key");
    let hello_again = VFS.get_inode(&hello_key).expect("VFS get_inode failed");
    kprintln!("got inode by key");
    assert_eq!(hello_again.get_inumber(), hello.get_inumber());

    let hello_again_bytes = read_text(&hello_again, 32);
    assert_eq!(
        core::str::from_utf8(&hello_again_bytes).unwrap(),
        "ext2 hello world"
    );

    // 4) Creating a new inode through the directory vnode works, and the new file
    // can be looked up, written, and read back.
    // the reason for this match is when running this test multiple times in a row the file
    // will persist on disk
    let created = match root.lookup("vfs-created.txt") {
        Ok(node) => node,
        Err(_) => root
            .create_child("vfs-created.txt", INodeType::File)
            .expect("create_inode failed"),
    };
    kprintln!("created file");

    let payload = b"created through vfs";
    let written = created
        .write_unaligned(0, payload)
        .expect("write_unaligned failed");
    assert_eq!(written, payload.len());
    kprintln!("wrote created file");

    let created_lookup = root
        .lookup("vfs-created.txt")
        .expect("created file not visible from parent dir");
    kprintln!("lookup created file");

    let created_bytes = read_text(&created_lookup, payload.len());
    assert_eq!(&created_bytes[..], payload);

    kprintln!("vfs/ext2 integration checks passed");
});
