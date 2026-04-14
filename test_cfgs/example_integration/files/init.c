void _start(void) {
  int x[1000];
  for (int i = 0; i < 1; i++) {
    x[i] = i;
  }
  asm("svc #0");
  while (1) {
  }
}
