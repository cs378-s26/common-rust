# System Calls

## How it works

When the program makes a syscall through svc or syscall, it will interrupt into its respective exception handler and then save the state of the interrupt as an event labeled syscall. This event is added to the event queue and the calling thread is put to sleep until the call is completed. The event handler receives the event, runs the code for that syscall, writes the result back into the saved registers, and wakes the thread up to continue.

## File locations

- **src/arch/aarch64/exceptions.rs**: This is the entry point for ARM. It handles the `SVC` instruction, saves the register state, and calls `push_event`. (Note: for ARM, we have to manually increment the PC by 4 so the program doesn't just run the same syscall again when it wakes up).
- **src/arch/x86_64/interrupt.rs**: Same as above but for x86.
- **src/event.rs**: Handles the `Syscall` event, grabs the saved context from the thread, and passes it to the handler.
- **src/syscall/mod.rs**: It contains a large match statement that looks at the syscall number and calls the respective functions. Look across the syscall directory for the different files containing the system calls (ie. `fs.rs` or `process.rs`).

## Important stuff to know

- **SyscallContext**: This is a trait that lets the generic handler code work on both ARM and x86. It has methods to get arguments (0 through 5) and set the return value in the correct register.
- **numbers.rs**: This file defines the unique ID for every syscall. x86-64 and Aarch64 do _not_ have the same system call numbers and do not support the same system calls.
- **Context**: The `Context` struct holds the register state of a thread. When a syscall happens, the handler modifies this struct (like setting the return value in `x0` or `rax`) before the thread starts running again.
