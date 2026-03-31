#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_common::test_runner)]

kernel_common::integration_test!({
    use core::sync::atomic::{AtomicU64, Ordering};
    use kernel_common::arch::{Arch, ArchTrait};
    use kernel_common::process::Process;
    use kernel_common::ramfs::{RAMFilesystem, init_test_ramfs};
    use kernel_common::vfs::{INodeKey, VFS};

    static LATCH: AtomicU64 = AtomicU64::new(0);

    fn test01() {
        LATCH.fetch_add(1, Ordering::SeqCst);
        let process = Process::new();
        Process::run(process.clone(), move || {
            let x = process
                .virtual_memory
                .mmap(
                    Some((INodeKey::new(0, 1), 0, None)),
                    Arch::PAGE_SIZE,
                    true,
                    None,
                )
                .unwrap();
            let y = process
                .virtual_memory
                .mmap(
                    Some((INodeKey::new(0, 1), 0, None)),
                    Arch::PAGE_SIZE,
                    true,
                    None,
                )
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
        let process = Process::new();
        Process::run(process.clone(), move || {
            let x = process
                .virtual_memory
                .mmap(
                    Some((INodeKey::new(0, 2), 0, None)),
                    Arch::PAGE_SIZE,
                    true,
                    None,
                )
                .unwrap();
            let y = process
                .virtual_memory
                .mmap(
                    Some((INodeKey::new(0, 2), 0, Some(Arch::PAGE_SIZE + 2))),
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
        let process = Process::new();
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

    fn test04() {
        LATCH.fetch_add(3, Ordering::SeqCst);
        let process = Process::new();
        Process::run(process.clone(), move || {
            let file_shared = process
                .virtual_memory
                .mmap(Some((INodeKey::new(0, 1), 0, None)), 4096, true, None)
                .unwrap();
            let file_private = process
                .virtual_memory
                .mmap(Some((INodeKey::new(0, 1), 0, None)), 4096, false, None)
                .unwrap();
            let anon_shared = process.virtual_memory.mmap(None, 4096, true, None).unwrap();
            let anon_private = process
                .virtual_memory
                .mmap(None, 4096, false, None)
                .unwrap();
            let new_process = process.fork();
            unsafe {
                *(anon_shared as *mut u8) = b'x';
                *(anon_private as *mut u8) = b'x';
                *(file_shared as *mut u8) = b'x';
                *(file_private as *mut u8) = b'x';
            }
            Process::run(new_process, move || {
                unsafe {
                    *(anon_shared as *mut u8) = b'y';
                    *(anon_private as *mut u8) = b'y';
                    *(file_shared as *mut u8) = b'y';
                    *(file_private as *mut u8) = b'y';
                }
                LATCH.fetch_sub(1, Ordering::SeqCst);
                unsafe {
                    assert!(*(anon_shared as *mut u8) == b'y');
                    assert!(*(anon_private as *mut u8) == b'y');
                    assert!(*(anon_shared as *mut u8) == b'y');
                    assert!(*(anon_private as *mut u8) == b'y');
                }
                LATCH.fetch_sub(1, Ordering::SeqCst);
            });
            while LATCH.load(Ordering::SeqCst) > 2 {}
            unsafe {
                assert!(*(anon_shared as *mut u8) == b'y');
                assert!(*(anon_private as *mut u8) == b'x');
                assert!(*(file_shared as *mut u8) == b'y');
                assert!(*(file_private as *mut u8) == b'x');
            }
            while LATCH.load(Ordering::SeqCst) > 1 {}
            unsafe { *(file_private as *mut u8) = b'c' };
            LATCH.fetch_sub(1, Ordering::SeqCst);
        });
        while LATCH.load(Ordering::SeqCst) > 0 {}
    }

    let fs = RAMFilesystem::new();
    init_test_ramfs(fs.clone());
    VFS.mount(fs);
    test01();
    test02();
    test03();
    test04();
});
