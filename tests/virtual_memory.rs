#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_common::test_runner)]

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
    };

    static LATCH: AtomicU64 = AtomicU64::new(0);

    fn file(name: &str) -> INodeKey {
        VFS.get_root()
            .unwrap()
            .lookup(name)
            .unwrap()
            .get_inode_key()
            .unwrap()
    }

    fn test01() {
        LATCH.fetch_add(1, Ordering::SeqCst);
        let process = Process::new().expect("failed to create process");
        Process::run(process.clone(), move || {
            let x = process
                .virtual_memory
                .mmap(Some((file("cat"), 0, None)), Arch::PAGE_SIZE, true, None)
                .unwrap();
            let y = process
                .virtual_memory
                .mmap(Some((file("cat"), 0, None)), Arch::PAGE_SIZE, true, None)
                .unwrap();
            assert!(x != y);
            unsafe {
                assert!(*(y as *const u8) == b'c');
                *(x as *mut u8) = b'b';
                assert!(*(y as *const u8) == b'b');
                *(y as *mut u8) = b'a';
                assert!(*(x as *const u8) == b'a');
                *(x as *mut u8) = b'c';
            };
            LATCH.fetch_sub(1, Ordering::SeqCst);
        });
        while LATCH.load(Ordering::SeqCst) > 0 {}
    }

    fn test02() {
        LATCH.fetch_add(1, Ordering::SeqCst);
        let process = Process::new().expect("failed to create process");
        Process::run(process.clone(), move || {
            let x = process
                .virtual_memory
                .mmap(Some((file("cats"), 0, None)), Arch::PAGE_SIZE, true, None)
                .unwrap();
            let y = process
                .virtual_memory
                .mmap(
                    Some((file("cats"), 0, Some(Arch::PAGE_SIZE + 2))),
                    Arch::PAGE_SIZE * 3,
                    false,
                    None,
                )
                .unwrap();
            assert!(x != y);
            unsafe {
                // COW for first page
                assert!(*(y as *const u8) == b'c');
                *(x as *mut u8) = b'b';
                assert!(*(y as *const u8) == b'b');
                *(y as *mut u8) = b'l';
                assert!(*(x as *const u8) == b'b');
                *(x as *mut u8) = b'c';

                // COR for second page
                assert!(*((y + Arch::PAGE_SIZE) as *const u8) == b'c');
                *((x + Arch::PAGE_SIZE) as *mut u8) = b'l';
                assert!(*((y + Arch::PAGE_SIZE) as *const u8) == b'c');
                *((y + Arch::PAGE_SIZE) as *mut u8) = b'm';
                assert!(*((x + Arch::PAGE_SIZE) as *const u8) == b'l');
                *((x + Arch::PAGE_SIZE) as *mut u8) = b'c';
                for i in 3..Arch::PAGE_SIZE {
                    assert!(*((y + Arch::PAGE_SIZE + i) as *const u8) == 0);
                }

                // Blank third page
                for i in 0..Arch::PAGE_SIZE {
                    assert!(*((y + 2 * Arch::PAGE_SIZE + i) as *const u8) == 0);
                }
            };
            LATCH.fetch_sub(1, Ordering::SeqCst);
        });
        while LATCH.load(Ordering::SeqCst) > 0 {}
    }

    fn test03() {
        LATCH.fetch_add(1, Ordering::SeqCst);
        let process = Process::new().expect("failed to create process");
        Process::run(process.clone(), move || {
            let x = process
                .virtual_memory
                .mmap(None, Arch::PAGE_SIZE, false, None)
                .unwrap();
            unsafe {
                *(x as *mut u8) = b'd';
                *((x + 1) as *mut u8) = b'o';
                *((x + 2) as *mut u8) = b'g';

                assert!(*(x as *const u8) == b'd');
                assert!(*((x + 1) as *const u8) == b'o');
                assert!(*((x + 2) as *const u8) == b'g');
                for i in 3..Arch::PAGE_SIZE {
                    assert!(*((x + i) as *const u8) == 0);
                }
            };
            LATCH.fetch_sub(1, Ordering::SeqCst);
        });
        while LATCH.load(Ordering::SeqCst) > 0 {}
    }

    let mut block_devices = BLOCK_DEVICES.lock();
    let fs = Ext2::new_from_block_devices(&mut block_devices)
        .expect("ext2 filesystem not found on attached block devices");
    drop(block_devices);

    let root = fs.get_root().unwrap();

    let cat;
    if let Ok(node) = root.create_child("cat", INodeType::File) {
        cat = node;
    } else {
        cat = root.lookup("cat").unwrap();
    }
    cat.write_unaligned(0, "cat".as_bytes()).unwrap();

    let cats;
    if let Ok(node) = root.create_child("cats", INodeType::File) {
        cats = node;
    } else {
        cats = root.lookup("cats").unwrap();
    }
    cats.write_unaligned(0, "cats".repeat(Arch::PAGE_SIZE).as_bytes())
        .unwrap();

    let _ = VFS.mount(fs, &["/"]).unwrap();
    test01();
    test02();
    test03();
    kprintln!("Virtual memory done!");
});
