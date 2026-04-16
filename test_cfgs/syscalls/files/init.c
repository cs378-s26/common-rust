void start(void) {
    long result = syscall1(1, 42); // Example syscall number and argument
    // Handle the result as needed (e.g., store it, print it, etc.)
    __asm__ volatile("mov x7, %x0\n"
      "svc #0\n"
      :
      : "r"(result)
      : "x7"
      );
    while (1) {}
}

inline long syscall1(long syscall_no, long arg0) {
    register long x8 asm("x8") = syscall_no;
    register long x0 asm("x0") = arg0;

    asm volatile(
        "svc #0"
        : "+r"(x0)         // x0 is input arg0, then overwritten with return value
        : "r"(x8)
        : "memory"
    );

    return x0;
}