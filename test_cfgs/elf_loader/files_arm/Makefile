CFLAGS = -std=c99 -ffreestanding -nostdlib -g -O0 -Wall -Werror

all: init

%.o: %.c Makefile
	aarch64-linux-gnu-gcc -c $(CFLAGS) $< -o $@

init: init.o
	aarch64-linux-gnu-ld -m aarch64elf -e start -Ttext=0x400000 -o init init.o
	rm -f *.o

clean:
	rm -f init *.o
