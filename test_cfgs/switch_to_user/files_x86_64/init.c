void start(void) {
  int x[3000];
  for (int i = 0; i < 3000; i++) {
    x[i] = i;
  }
  long long sum = 1;
  for (int i = 0; i < 12; i++) {
    sum += x[i];
  }
  __asm__ volatile(
      "int $0x80\n"
      :
      : "a"(sum)
      : "memory");
  while (1) {
  }
}
