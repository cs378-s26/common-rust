#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_common::test_runner)]

kernel_common::integration_test!({
    use kernel_common::{
        devices::discovery::BLOCK_DEVICES,
        elf::ElfLoader,
        fs::{
            ext2::Ext2,
            vfs::{Filesystem, VFS},
        },
        print::kprintln,
        process::Process,
        sync::MutexLike,
        thread::spawn_user_thread,
    };
    let mut block_devices = BLOCK_DEVICES.lock();
    let fs = Ext2::new_from_block_devices(&mut block_devices)
        .expect("ext2 filesystem not found on attached block devices");
    drop(block_devices);
    VFS.set_root(fs.get_root().unwrap()).unwrap();
    VFS.mount(fs);

    let process = Process::new().expect("failed to create process");
    let root = VFS.get_root().unwrap();
    let node = root.lookup("init").unwrap();
    let start_address = ElfLoader::load(node, &process).expect("Failed to load ELF file.");
    let stack = process
        .virtual_memory
        .mmap(None, 4096 * 4, false, None)
        .unwrap();
    spawn_user_thread(&process, start_address as usize, stack + 4096 * 4);
    let exit_code = process.exit_code.get();
    if exit_code == 0 {
        kprintln!("User thread exited successfully.");
    } else {
        kprintln!("User thread exited with code {}.", exit_code);
    }
});
