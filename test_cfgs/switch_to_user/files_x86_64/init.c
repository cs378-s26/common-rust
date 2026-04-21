static inline long syscall_1_arg(long n, long a0) {
    long ret;
    __asm__ volatile(
        "int $0x80"
        : "=a"(ret)
        : "a"(n), "D"(a0)
        : "memory"
    );
    return ret;
}

static inline long syscall_0_arg(long n) {
    long ret;
    __asm__ volatile(
        "int $0x80"
        : "=a"(ret)
        : "a"(n)
        : "memory"
    );
    return ret;
}

void start(void) {
    long pid = syscall_0_arg(39);

    if (pid != 1) {
        syscall_1_arg(60, 1);
    } else {
        syscall_1_arg(60, 0);
    }

    while (1) {}
}