void _start(void) {
  int x[3000];
  for (int i = 0; i < 3000; i++) {
    x[i] = i;
  }
  long long sum = 1;
  for (int i = 0; i < 12; i++) {
    sum += x[i];
  }
  asm("mov x8, %x0\n"
      "svc #0\n"
      :
      : "r"(sum)
      : "x8"
      );
  while (1) {
  }
}
