CFLAGS = -std=c99 -m64 -nostdlib -g -O0 -Wall -Werror

all : init

%.o :  Makefile %.c
	gcc -c $(CFLAGS) $*.c

init : Makefile init.o
	ld -m elf_x86_64 -e start -Ttext=0xEFA00000 -o init init.o
	rm -f *.o
	
clean ::
	rm -f init
