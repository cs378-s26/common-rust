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
    VFS.mount(ext2.clone(), &["/"]).expect("Failed to mount root");

    let process = Process::new().unwrap();
    Process::run(process.clone(), move || {
        let thread = THIS_THREAD.get().unwrap().upgrade().unwrap();
        
        // Use syscall numbers from numbers module
        use kernel_common::syscall::number;
        use kernel_common::syscall::AT_FDCWD;

        kprintln!("Testing syscalls exhaustively...");

        // 1. Basic Create, Write, Close, Open, Read, Close
        let path = "test_exhaustive.txt\0";
        let fd = call_syscall(&thread, number::OPENAT, AT_FDCWD as u64, path.as_ptr() as u64, 64, 0);
        assert!(fd >= 3);
        kprintln!("1. openat O_CREAT passed, fd: {}", fd);

        let data = "exhaustive test data";
        let written = call_syscall(&thread, number::WRITE, fd, data.as_ptr() as u64, data.len() as u64, 0);
        assert_eq!(written, data.len() as u64);
        kprintln!("2. write passed");

        let res = call_syscall(&thread, number::CLOSE, fd, 0, 0, 0);
        assert_eq!(res, 0);
        kprintln!("3. close passed");

        let fd2 = call_syscall(&thread, number::OPENAT, AT_FDCWD as u64, path.as_ptr() as u64, 0, 0);
        assert!(fd2 >= 3);
        kprintln!("4. openat read passed, fd: {}", fd2);

        let mut buf = [0u8; 32];
        let read_bytes = call_syscall(&thread, number::READ, fd2, buf.as_mut_ptr() as u64, data.len() as u64, 0);
        assert_eq!(read_bytes, data.len() as u64);
        assert_eq!(&buf[..data.len()], data.as_bytes());
        kprintln!("5. read passed");

        // 2. Test reading past EOF
        let read_eof = call_syscall(&thread, number::READ, fd2, buf.as_mut_ptr() as u64, 32, 0);
        assert_eq!(read_eof, 0);
        kprintln!("6. read past EOF returned 0");

        call_syscall(&thread, number::CLOSE, fd2, 0, 0, 0);

        // 3. Test opening existing file from image
        let hello_path = "hello.txt\0";
        let fd_hello = call_syscall(&thread, number::OPENAT, AT_FDCWD as u64, hello_path.as_ptr() as u64, 0, 0);
        assert!(fd_hello >= 3);
        let mut hello_buf = [0u8; 32];
        let hello_read = call_syscall(&thread, number::READ, fd_hello, hello_buf.as_mut_ptr() as u64, 32, 0);
        assert!(hello_read > 0);
        kprintln!("7. open/read existing file passed: {}", core::str::from_utf8(&hello_buf[..hello_read as usize]).unwrap());
        call_syscall(&thread, number::CLOSE, fd_hello, 0, 0, 0);

        // 4. Test directory fd and relative path
        let dir_path = "dir\0";
        let fd_dir = call_syscall(&thread, number::OPENAT, AT_FDCWD as u64, dir_path.as_ptr() as u64, 0, 0);
        assert!(fd_dir >= 3);
        kprintln!("8. open directory passed, fd: {}", fd_dir);

        let nested_path = "nested.txt\0";
        let fd_nested = call_syscall(&thread, number::OPENAT, fd_dir, nested_path.as_ptr() as u64, 0, 0);
        assert!(fd_nested >= 3);
        let mut nested_buf = [0u8; 32];
        let nested_read = call_syscall(&thread, number::READ, fd_nested, nested_buf.as_mut_ptr() as u64, 32, 0);
        assert!(nested_read > 0);
        kprintln!("9. openat relative to dir fd passed: {}", core::str::from_utf8(&nested_buf[..nested_read as usize]).unwrap());
        
        // 5. Create new file relative to directory fd
        let new_rel_path = "new_relative.txt\0";
        let fd_new_rel = call_syscall(&thread, number::OPENAT, fd_dir, new_rel_path.as_ptr() as u64, 64, 0);
        assert!(fd_new_rel >= 3);
        let rel_data = "relative file data";
        call_syscall(&thread, number::WRITE, fd_new_rel, rel_data.as_ptr() as u64, rel_data.len() as u64, 0);
        kprintln!("10. create/write file relative to dir fd passed");

        call_syscall(&thread, number::CLOSE, fd_new_rel, 0, 0, 0);
        call_syscall(&thread, number::CLOSE, fd_nested, 0, 0, 0);
        call_syscall(&thread, number::CLOSE, fd_dir, 0, 0, 0);

        // 6. Error cases
        // Non-existent file without O_CREAT
        let no_file_path = "no_such_file.txt\0";
        let res_no_file = call_syscall(&thread, number::OPENAT, AT_FDCWD as u64, no_file_path.as_ptr() as u64, 0, 0);
        assert_eq!(res_no_file as i64, -1);
        kprintln!("11. open non-existent file failed as expected");

        // Invalid fd for close
        let res_close_inv = call_syscall(&thread, number::CLOSE, 999, 0, 0, 0);
        assert_eq!(res_close_inv as i64, -1);
        kprintln!("12. close invalid fd failed as expected");

        // Invalid fd for read
        let res_read_inv = call_syscall(&thread, number::READ, 999, buf.as_mut_ptr() as u64, 32, 0);
        assert_eq!(res_read_inv as i64, -1);
        kprintln!("13. read invalid fd failed as expected");

        kprintln!("All exhaustive syscall tests passed!");
        DONE.store(1, Ordering::SeqCst);
    });

    while DONE.load(Ordering::SeqCst) == 0 {}
    kprintln!("Syscall test complete.");
});
