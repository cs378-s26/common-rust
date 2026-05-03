#[cfg(target_arch = "aarch64")]
pub mod number {
    pub const MKDIRAT: u64 = 34;
    pub const UNLINKAT: u64 = 35;
    pub const FACCESSAT: u64 = 48;
    pub const OPENAT: u64 = 56;
    pub const CLOSE: u64 = 57;
    pub const PIPE2: u64 = 59;
    pub const READ: u64 = 63;
    pub const WRITE: u64 = 64;
    pub const PSELECT6: u64 = 72;
    pub const PPOLL: u64 = 73;
    pub const NEWFSTATAT: u64 = 79;
    pub const NANOSLEEP: u64 = 101;
    pub const CLONE: u64 = 220;
    pub const EXIT: u64 = 93;
    pub const GETPID: u64 = 172;
}

#[cfg(target_arch = "x86_64")]
pub mod number {
    pub const READ: u64 = 0;
    pub const WRITE: u64 = 1;
    pub const OPEN: u64 = 2;
    pub const CLOSE: u64 = 3;
    pub const STAT: u64 = 4;
    pub const LSTAT: u64 = 6;
    pub const POLL: u64 = 7;
    pub const ACCESS: u64 = 21;
    pub const PIPE: u64 = 22;
    pub const SELECT: u64 = 23;
    pub const NANOSLEEP: u64 = 35;
    pub const CLONE: u64 = 56;
    pub const FORK: u64 = 57;
    pub const VFORK: u64 = 58;
    pub const MKDIR: u64 = 83;
    pub const RMDIR: u64 = 84;
    pub const UNLINK: u64 = 87;

    pub const OPENAT: u64 = 257;
    pub const MKDIRAT: u64 = 258;
    pub const FACCESSAT: u64 = 269;
    pub const PSELECT6: u64 = 270;
    pub const PPOLL: u64 = 271;
    pub const NEWFSTATAT: u64 = 262;
    pub const UNLINKAT: u64 = 263;
    pub const PIPE2: u64 = 293;
    pub const EXIT: u64 = 60;
    pub const GETPID: u64 = 39;
}

// Constants for syscall translation via wrappers
#[cfg(target_arch = "x86_64")]
pub mod wrapper_constants {
    pub const AT_FDCWD: i32 = -100;
    pub const AT_SYMLINK_NOFOLLOW: i32 = 0x100;
    pub const SIGCHLD: u64 = 17;
    pub const CLONE_VM: u64 = 0x00000100;
    pub const CLONE_VFORK: u64 = 0x00004000;
}
