#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_common::test_runner)]

#[cfg(target_arch = "aarch64")]
kernel_common::integration_test!({
    use core::sync::atomic::{AtomicU64, Ordering};

    use kernel_common::{
        arch::{Arch, ArchTrait},
        devices::discovery::BLOCK_DEVICES,
        fs::{
            ext2::Ext2,
            vfs::{Filesystem, INodeKey, INodeType, VFS},
        },
        print::kprintln,
        process::Process,
        sync::MutexLike,
        thread::{spawn_thread_but_based, yield_thread},
    };
    let mut block_devices = BLOCK_DEVICES.lock();
    let fs = Ext2::new_from_block_devices(&mut block_devices)
        .expect("ext2 filesystem not found on attached block devices");
    drop(block_devices);
    VFS.set_root(fs.get_root().unwrap()).unwrap();
    VFS.mount(fs);

    let process = Process::new();
    let root = VFS.get_root().unwrap();
    let node = root.lookup("init").unwrap();
    let _ = process
        .virtual_memory
        .mmap(
            Some((node.get_inode_key().unwrap(), 0, None)),
            4096,
            false,
            Some(0x40000),
        )
        .unwrap();
    let stack = process
        .virtual_memory
        .mmap(None, 4096 * 4, false, None)
        .unwrap();
    spawn_thread_but_based(&process, 0x40000, stack + 4096 * 4);
    loop {
        if let x = process.exit_code.load(Ordering::SeqCst)
            && x != 0
        {
            kprintln!("{}", x);
            break;
        }
        yield_thread();
    }
});
