.macro SAVE_REGS
    sub sp, sp, #17 * 16

    stp x0, x1, [sp, #16 * 0]
    stp x2, x3, [sp, #16 * 1]
    stp x4, x5, [sp, #16 * 2]
    stp x6, x7, [sp, #16 * 3]
    stp x8, x9, [sp, #16 * 4]
    stp x10, x11, [sp, #16 * 5]
    stp x12, x13, [sp, #16 * 6]
    stp x14, x15, [sp, #16 * 7]
    stp x16, x17, [sp, #16 * 8]
    stp x18, x19, [sp, #16 * 9]
    stp x20, x21, [sp, #16 * 10]
    stp x22, x23, [sp, #16 * 11]
    stp x24, x25, [sp, #16 * 12]
    stp x26, x27, [sp, #16 * 13]
    stp x28, x29, [sp, #16 * 14]

    // move special registers into available spaces
    mrs x0, ELR_EL1
    mrs x1, SPSR_EL1
    mrs x2, ESR_EL1

    stp x30, x0, [sp, #16 * 15]  
    stp x1, x2, [sp, #16 * 16]   
.endm

.macro RESTORE_REGS
    // Restore special registers first
    ldp x30, x0, [sp, #16 * 15]   // x30=LR, x0=ELR_EL1
    ldp x1, x2,  [sp, #16 * 16]   // x1=SPSR_EL1, x2=ESR_EL1 (discarded)
    msr ELR_EL1,  x0
    msr SPSR_EL1, x1

    // Restore general purpose registers
    ldp x0,  x1,  [sp, #16 * 0]
    ldp x2,  x3,  [sp, #16 * 1]
    ldp x4,  x5,  [sp, #16 * 2]
    ldp x6,  x7,  [sp, #16 * 3]
    ldp x8,  x9,  [sp, #16 * 4]
    ldp x10, x11, [sp, #16 * 5]
    ldp x12, x13, [sp, #16 * 6]
    ldp x14, x15, [sp, #16 * 7]
    ldp x16, x17, [sp, #16 * 8]
    ldp x18, x19, [sp, #16 * 9]
    ldp x20, x21, [sp, #16 * 10]
    ldp x22, x23, [sp, #16 * 11]
    ldp x24, x25, [sp, #16 * 12]
    ldp x26, x27, [sp, #16 * 13]
    ldp x28, x29, [sp, #16 * 14]

    // Deallocate the whole frame at once
    add sp, sp, #17 * 16
.endm


.section .text



// Exception vector table
// Align by 2^11 bytes, as demanded by ARMv8-A. Same as ALIGN(2048) in an ld script.
.align 11

.global exception_vector_table
exception_vector_table:
    // Current EL with SP0
    .align 7
    b c_elx_sync_handler
    .align 7
    b c_default_irq_handler
    .align 7
    b .
    .align 7
    b .
    
    // Current EL with SPx
	// kernel faults
    .align 7
    b c_elx_sync_handler
    .align 7
    b c_elx_irq_handler
    .align 7
    b .
    .align 7
    b .
    
    // Lower EL (AArch64)
	// syscalls
    .align 7
    b .
    .align 7
    b c_default_irq_handler
    .align 7
    b .
    .align 7
    b .
    
    // Lower EL (AArch32)
    .align 7
    b .
    .align 7
    b c_default_irq_handler
    .align 7
    b .
    .align 7
    b .


c_default_irq_handler:
    SAVE_REGS
    mov x0, sp
    bl unimplemented
    RESTORE_REGS
    eret

c_elx_sync_handler:
    SAVE_REGS
    mov x0, sp
    bl current_elx_synchronous
    RESTORE_REGS
    eret

c_elx_irq_handler:
    SAVE_REGS
    mov x0, sp
    bl current_elx_irq
    RESTORE_REGS
    eret