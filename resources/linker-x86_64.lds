OUTPUT_FORMAT(elf64-x86-64)
OUTPUT_ARCH(i386:x86-64)
ENTRY(system_main)

PHDRS
{
    limine_requests PT_LOAD;
    text PT_LOAD FLAGS((1 << 0) | (1 << 2));
    rodata PT_LOAD FLAGS((1 << 2));
    data PT_LOAD FLAGS((1 << 1) | (1 << 2));
}

SECTIONS
{
    . = 0xffffffff80000000;

    _marker_kernel_start = .;

    /* limine */

    _marker_limine_request_start = .;
    .limine_requests : {
        KEEP(*(.limine_requests_start))
        KEEP(*(.limine_requests))
        KEEP(*(.limine_requests_end))
    } :limine_requests
    _marker_limine_request_end = .;

    . = ALIGN(CONSTANT(MAXPAGESIZE));

    /* .text */

    _marker_text_start = .;
    .text :
    {
        *(.text .text.*)
    } :text
    _marker_text_end = .;

    . = ALIGN(CONSTANT(MAXPAGESIZE));

    /* rodata */

    _marker_rodata_start = .;
    .rodata :
    {
        *(.rodata .rodata.*)
    } :rodata

    /* got */

    _marker_got_start = .;
    .got :
    {
        *(.got)
    } :rodata
    _marker_got_end = .;

    /* cpu local template */

    . = ALIGN(CONSTANT(MAXPAGESIZE));

    _marker_cpu_local_template_start = .;

    .cpu_local_template :
    {
        *(.cpu_local)
    } :rodata

    . = ALIGN(CONSTANT(MAXPAGESIZE));

    _marker_cpu_local_template_end = .;

    /* thread local template */

    . = ALIGN(CONSTANT(MAXPAGESIZE));

    _marker_thread_local_template_start = .;

    .thread_local_template :
    {
        *(.thread_local)
    } :rodata

    . = ALIGN(CONSTANT(MAXPAGESIZE));

    _marker_thread_local_template_end = .;

    _marker_rodata_end = .;

    /* data */

    . = ALIGN(CONSTANT(MAXPAGESIZE));

    _marker_data_start = .;

    .data :
    {
        *(.data .data.*)
    } :data

    .bss :
    {
        *(COMMON)
        *(.bss .bss.*)
    } :data

    _marker_data_end = .;
    _marker_kernel_end = .;

    /DISCARD/ : {
        *(.comment)
        *(.eh_frame*)
        *(.note .note.*)
    }
}
