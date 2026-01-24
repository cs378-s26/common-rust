ENTRY(system_main)

/* Loadable segments */
PHDRS
{
    limine_requests PT_LOAD;  /* limine requests */
    text PT_LOAD;             /* code */
    rodata PT_LOAD;           /* read-only data */
    data PT_LOAD;             /* data, bss, ... */
}

SECTIONS
{

    /*******************/
    /* Limine Requests */
    /*******************/

    .limine_requests 0xffffffff80000000 : ALIGN(4096) {
        /* Start request marker, must be first */
        QUAD(0xf6b8f4b39de7d1ae)
        QUAD(0xfab91a6940fcb9cf)
        QUAD(0x785c6ed015d3e316)
        QUAD(0x181e920a7852b9d9)
        /* The actual requests */
        KEEP(*(.limine_requests))
        /* End requests marker, must be last */
        QUAD(0xadc0e0531bb10d03)
        QUAD(0x9572709f31764c62)
    } :limine_requests

    /********/
    /* Text */
    /********/
    
    .text : ALIGN(4096) {
        *(.text .text.*)
    } :text

    /******************/
    /* Read only data */
    /******************/

    .rodata : ALIGN(4096) {
        *(.rodata .rodata.*)      /* string literals, ... */
    } :rodata

    .note.gnu.build-id : {
        *(.note.gnu.build-id)     /* optional build uuid */
    } :rodata

    .init_array : {
        _init_array_start = .;
        *(.init_array .init_array.*)
        _init_array_end = .;
    } :rodata

    /* not exactly standard behavior but we know that the kernel only
     * supports the static-exec model (see common.flags)
     */
    .tls : ALIGN(16) {
        __tls_start__ = .;
        *(.tdata*)
        *(.tbss*)
        . = ALIGN(16);
        __tls_end__ = .;
    } :rodata

    /********/
    /* Data */
    /********/

    /* initialized data */
    .data : ALIGN(4096) {
        *(.data .data.*)
    } :data


    /* zero-filled data */
    /* has to be last in order to avoid adding extra zero's to the file */
    .bss : ALIGN(4096) {
        *(.bss .bss.*)
        *(COMMON)
	/* non-standard: give the kernel an initial heap */
	. = ALIGN(16);
	__heap_used__ = .; /* divisible by 16 */
	. = . + 8;
	__heap_start__ = .;     /* divisible by 8 but not 16 */
	. = . + 6 * 1024 * 1024;
	__heap_end__ = .;
    } :data


}

