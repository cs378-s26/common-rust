#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_common::test_runner)]

kernel_common::integration_test!({
    extern crate alloc;

    use core::sync::atomic::{AtomicU64, Ordering};

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
    };

    #[cfg(target_arch = "x86_64")]
    fn read_entry(entry: u64) {
        kprintln!("First 26 bytes:");
        for i in 0..26 {
            kprintln!("{:#x}: {:02x}", entry + i, unsafe {
                *((entry + i) as *const u8)
            });
        }
    }

    #[cfg(target_arch = "aarch64")]
    fn read_entry(entry: u64) {
        kprintln!("First 8 instructions:");
        for i in 0..8 {
            kprintln!("{:#x}: {:08x}", entry + i * 4, unsafe {
                *((entry + i * 4) as *const u32)
            });
        }
    }

    static DONE_COUNT: AtomicU64 = AtomicU64::new(0);

    let process = Process::new();
    Process::run(process.clone(), move || {
        kprintln!("Inside process.");
        // Mount file system (stolen from ext2_vfs.rs).
        let mut block_devices = BLOCK_DEVICES.lock();
        let ext2 = Ext2::new_from_block_devices(&mut block_devices)
            .expect("ext2 filesystem not found on attached block devices");
        drop(block_devices);
        let root = ext2.get_root().expect("failed to get ext2 root");
        let _fs_id = VFS.mount(ext2.clone());
        VFS.set_root(root.clone()).ok();
        let root = VFS.get_root().expect("root not set");
        kprintln!("Mounted ext2 and set root.");

        // Find ELF file.
        let hello = root.lookup("init").expect("File 'init' not found.");
        kprintln!("Found ELF file.");

        // Load ELF file.
        let entry = ElfLoader::load(hello, &process).expect("Failed to load ELF file.");
        kprintln!("Loaded ELF file. Entry point: {:#x}", entry);

        // Read from entry.
        read_entry(entry);

        kprintln!("\nProcess complete.");
        DONE_COUNT.fetch_add(1, Ordering::SeqCst);
    });

    while DONE_COUNT.load(Ordering::SeqCst) < 1 {}
    kprintln!("ELF loader test complete.");
});
