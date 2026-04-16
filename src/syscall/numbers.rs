#[cfg(target_arch = "aarch64")]
pub mod number {
    pub const MKDIRAT: usize = 34;
    pub const UNLINKAT: usize = 35;
    pub const FACCESSAT: usize = 48;
    pub const OPENAT: usize = 56;
    pub const CLOSE: usize = 57;
    pub const PIPE2: usize = 59;
    pub const READ: usize = 63;
    pub const WRITE: usize = 64;
    pub const PSELECT6: usize = 72;
    pub const PPOLL: usize = 73;
    pub const NEWFSTATAT: usize = 79;
    pub const CLONE: usize = 220;
}

#[cfg(target_arch = "x86_64")]
pub mod number {
    pub const READ: usize = 0;
    pub const WRITE: usize = 1;
    pub const OPEN: usize = 2;
    pub const CLOSE: usize = 3;
    pub const STAT: usize = 4;
    pub const LSTAT: usize = 6;
    pub const POLL: usize = 7;
    pub const ACCESS: usize = 21;
    pub const PIPE: usize = 22;
    pub const SELECT: usize = 23;
    pub const CLONE: usize = 56;
    pub const FORK: usize = 57;
    pub const VFORK: usize = 58;
    pub const MKDIR: usize = 83;
    pub const RMDIR: usize = 84;
    pub const UNLINK: usize = 87;

    pub const OPENAT: usize = 257;
    pub const MKDIRAT: usize = 258;
    pub const FACCESSAT: usize = 269;
    pub const PSELECT6: usize = 270;
    pub const PPOLL: usize = 271;
    pub const NEWFSTATAT: usize = 262;
    pub const UNLINKAT: usize = 263;
    pub const PIPE2: usize = 293;
}

// Constants for syscall translation via wrappers
#[cfg(target_arch = "x86_64")]
pub mod wrapper_constants {
    pub const AT_FDCWD: i32 = -100;
    pub const AT_SYMLINK_NOFOLLOW: i32 = 0x100;
    pub const SIGCHLD: usize = 17;
    pub const CLONE_VM: usize = 0x00000100;
    pub const CLONE_VFORK: usize = 0x00004000;
}
