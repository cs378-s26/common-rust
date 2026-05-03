# Threading

- `kernel_main`'s job is mainly to set up threading it does this via the following sequence of steps:
    - One core initalizes a barrier & theading (namely the work queue) for the system. All cores wait on this barrier
    - Once the barrier is passed, all cores set up their idle thread and wait on another barrier. After the final barrier is passed, each core turns on preemption and starts running the scheduler loop (via `poll_tasks`)
## thread lifecycle

- threads are created via `spawn_thread` which takes a lambda and does the following:
    1. Creates a new thread & sets up the thread's context (via `setup_for_call` in the arch module)
        - Context is meant to be a snapshot of cpu state, so gp regs, flags, and sp
        - To set up the context for a call the kernel sets the instruction pointer to the start of a trampoline function (to be discussed).
        - This trampoline function takes a pointer to the passed in lambda and calls it, so this setup function also passed the lambda pointer to our trampoline
    2. Turns on yielding for the thread
    3. Adds the thread to the scheduler's run queue
- **the trampoline**:
    - small function which: 
        1. calls the passed in lambda
        2. calls `thread_exit` to clean up the thread and remove it from the scheduler (NOTE: not implemented yet)
    - you'll notice that the trampoline isn't what's passed in, instead we pass in a pointer to `thread_entry0` this is because the ABI for x86_64 requires that the address of our stack frame be 8 mod 16.
        - this is kind of cooked, since on aarch64 the ABI reuqires 0 mod 16
- **idle thread polling**: 
    - The idle thread sits there and polls for work
    - cores have a local_queue that gets drained to the global queue
    - which is then used to switch to tasks
- **task switching**:
    - done by suspend_impl which takes a target thread & an action to perform on the current one
    - this calls save_context on the current thread & context, and exits the thread while performing the action 
    - TODO: got lazy & didn't finish :P but pretty straight forward from here

## Sleeping

When a thread calls `sleep(ms)`, it blocks to a global sleep queue (implemented as an intrusive red-black tree sorted by wakeup time, calculated as the current system jiffy plus the passed parameter). A background thread periodically checks the sleep queue and wakes up threads whose wakeup time has come by moving them back to the run queue. Threads are thus guaranteed to sleep for at least the specified duration, but may sleep longer if the system is under heavy load or if the background thread is delayed.