.macro SAVE_REGS
    sub sp, sp, #18 * 16

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

    // when saving the sp on ARM, we need to know which one to save.
    // this code figures that out, and puts the result into x3.
    and x4, x1, #0b1111
    cmp x4, #0b0000
    b.eq 1f
    add x3, sp, #18 * 16
    mov x5, xzr
    b 2f
1:      
    mrs x3, sp_el0
    mrs x5, tpidr_el0
2:

    stp x3, x5, [sp, #16 * 17] // store stack pointer and user tcb pointer
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
    add sp, sp, #18 * 16
.endm

.section .text





// Exception vector table

// four levels, four types per level
// levels: 
//   Current EL with SP0 - exception taken at level x while using user stack pointer. very rare and atypical
//   Current EL with SPx - exception taken at level x while using stack pointer x. ex: fault within the kernel
//   Lower EL (AArch64)  - exception taken from EL0. ex: userspace syscall or userspace fault
//   Lower EL (AArch32)  - same as previous but for AArch32. We will not use.
// types (they follow this order in the vector):
//   Synchronous - caused by an instruction: svc, page fault, breakpoint, etc.
//   IRQ - Asynchronous interrupt from hardware: UART, GIC, DMA, etc.
//   Fast IRQ - higher priority than IRQ and meant for specific latency critical hardware interrupts
//   SError - system error that can't be attributed to a single instruction (which would be synchronous) but is likely fatal. ex. bus error or memory fault


// Align by 2^11 bytes, as demanded by ARMv8-A. Same as ALIGN(2048) in an ld script.
.align 11

.global exception_vector_table
exception_vector_table:
    // Current EL with SP0
    .align 7
    b c_elx_sync_handler
    .align 7
    b c_unimplemented_handler
    .align 7
    b c_unimplemented_handler
    .align 7
    b c_unimplemented_handler
    
    // Current EL with SPx
	// kernel faults
    .align 7
    b c_elx_sync_handler
    .align 7
    b c_elx_irq_handler
    .align 7
    b c_unimplemented_handler
    .align 7
    b c_unimplemented_handler
    
    // Lower EL (AArch64)
	// syscalls
    .align 7
    b c_el0_sync_handler
    .align 7
    b c_el0_irq_handler
    .align 7
    b c_unimplemented_handler
    .align 7
    b c_unimplemented_handler
    
    // Lower EL (AArch32)
    .align 7
    b c_unimplemented_handler
    .align 7
    b c_unimplemented_handler
    .align 7
    b c_unimplemented_handler
    .align 7
    b c_unimplemented_handler


c_unimplemented_handler:
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

c_el0_sync_handler:
    SAVE_REGS
    mov x0, sp
    bl el0_sync
    RESTORE_REGS
    eret

c_el0_irq_handler:
    SAVE_REGS
    mov x0, sp
    bl el0_irq
    RESTORE_REGS
    eret
