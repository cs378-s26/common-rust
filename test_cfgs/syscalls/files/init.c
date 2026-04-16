static inline long syscall1(long syscall_no, long arg0) {
    register long x8 __asm__("x8") = syscall_no;
    register long x0 __asm__("x0") = arg0;

    __asm__ volatile(
        "svc #0"
        : "+r"(x0)
        : "r"(x8)
        : "memory"
    );

    return x0;
}

void start(void) {
    long result = syscall1(1, 42);
    syscall1(2, result);


    while (1) {}
}