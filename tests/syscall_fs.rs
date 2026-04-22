#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_common::test_runner)]

kernel_common::integration_test!({
    extern crate alloc;
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicU64, Ordering};
    use kernel_common::{
        devices::discovery::BLOCK_DEVICES,
        fs::{
            ext2::Ext2,
            vfs::{Filesystem, VFS},
        },
        print::kprintln,
        process::Process,
        sync::MutexLike,
        syscall::{syscall_handler, SyscallContext},
        thread::{Thread, THIS_THREAD},
    };

    // Helper for calling syscalls from kernel space using the same handler
    struct TestSyscallContext {
        num: u64,
        args: [u64; 6],
        ret: u64,
    }

    impl SyscallContext for TestSyscallContext {
        fn syscall_number(&self) -> u64 { self.num }
        fn arg0(&self) -> u64 { self.args[0] }
        fn arg1(&self) -> u64 { self.args[1] }
        fn arg2(&self) -> u64 { self.args[2] }
        fn arg3(&self) -> u64 { self.args[3] }
        fn arg4(&self) -> u64 { self.args[4] }
        fn arg5(&self) -> u64 { self.args[5] }
        fn set_return_value(&mut self, ret: u64) { self.ret = ret; }
        fn is_user_address(&self, _ptr: u64) -> bool { true } // Treat all as user for test
    }

    fn call_syscall(thread: &Arc<Thread>, num: u64, a0: u64, a1: u64, a2: u64, a3: u64) -> u64 {
        let mut ctx = TestSyscallContext {
            num,
            args: [a0, a1, a2, a3, 0, 0],
            ret: 0,
        };
        syscall_handler(thread, &mut ctx);
        ctx.ret
    }

    static DONE: AtomicU64 = AtomicU64::new(0);

    // Mount file system
    let mut block_devices = BLOCK_DEVICES.lock();
    let ext2 = Ext2::new_from_block_devices(&mut block_devices)
        .expect("ext2 filesystem not found");
    drop(block_devices);
    VFS.mount(ext2.clone());
    VFS.set_root(ext2.get_root().unwrap()).unwrap();

    let process = Process::new().unwrap();
    Process::run(process.clone(), move || {
        let thread = THIS_THREAD.get().unwrap().upgrade().unwrap();
        
        // Use syscall numbers from numbers module
        use kernel_common::syscall::number;
        use kernel_common::syscall::AT_FDCWD;

        kprintln!("Testing syscalls...");

        // 1. Openat with O_CREAT
        let path = "test_syscall.txt\0";
        // O_CREAT is 64
        let fd = call_syscall(&thread, number::OPENAT, AT_FDCWD as u64, path.as_ptr() as u64, 64, 0);
        assert!(fd >= 3, "failed to open/create file, got fd {}", fd as i64);
        kprintln!("openat O_CREAT passed, fd: {}", fd);

        // 2. Write
        let data = "syscall write test";
        let written = call_syscall(&thread, number::WRITE, fd, data.as_ptr() as u64, data.len() as u64, 0);
        assert_eq!(written, data.len() as u64);
        kprintln!("write passed");

        // 3. Close
        let res = call_syscall(&thread, number::CLOSE, fd, 0, 0, 0);
        assert_eq!(res, 0);
        kprintln!("close passed");

        // 4. Openat again for reading
        let fd2 = call_syscall(&thread, number::OPENAT, AT_FDCWD as u64, path.as_ptr() as u64, 0, 0);
        assert!(fd2 >= 3);
        kprintln!("openat read passed, fd: {}", fd2);

        // 5. Read
        let mut buf = [0u8; 32];
        let read_bytes = call_syscall(&thread, number::READ, fd2, buf.as_mut_ptr() as u64, 32, 0);
        assert_eq!(read_bytes, data.len() as u64);
        assert_eq!(&buf[..data.len()], data.as_bytes());
        kprintln!("read passed: {}", core::str::from_utf8(&buf[..data.len()]).unwrap());

        // 6. Close
        let res2 = call_syscall(&thread, number::CLOSE, fd2, 0, 0, 0);
        assert_eq!(res2, 0);
        kprintln!("close passed again");

        kprintln!("All syscall tests passed!");
        DONE.store(1, Ordering::SeqCst);
    });

    while DONE.load(Ordering::SeqCst) == 0 {}
    kprintln!("Syscall test complete.");
});
