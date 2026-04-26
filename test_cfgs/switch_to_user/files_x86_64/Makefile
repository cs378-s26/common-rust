CFLAGS = -std=c99 -ffreestanding -nostdlib -g -O0 -Wall -Werror

all: init

%.o: %.c Makefile
	x86_64-linux-gnu-gcc -c $(CFLAGS) $< -o $@

init: init.o
	x86_64-linux-gnu-ld -m elf_x86_64 -e start -Ttext=0x400000 -o init init.o
	rm -f *.o

clean:
	rm -f init *.o
