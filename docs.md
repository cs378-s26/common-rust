# Various Notes on Ricky's Kernel

mostly focused on how the hardware interacts w/ the kernel.

## all arch functions called from kernel
### initialization functions
- [x] `crate::arch::initialize_mp` (caller: `system_main`; site: `src/main.rs:186`)
- [ ] `crate::arch::init_tty` (caller: `print::init_tty`; site: `src/print.rs:270`)
- [x] `crate::arch::Context::setup_kthread_context` (caller: `set_up_idle`; site: `src/thread.rs:206`)
### general info
- [x] `crate::arch::core_count` (caller: `kernel_main`; sites: `src/main.rs:199`, `src/main.rs:202`, `src/main.rs:214`)
- [ ] `crate::arch::read_cycle_counter` (caller: `kernel_main` spawned thread closure; sites: `src/main.rs:228`, `src/main.rs:229`)
### threading & interrupts
- [x] `crate::arch::irq_enable` (callers: `kernel_main`, `poll_tasks`; sites: `src/main.rs:246`, `src/thread.rs:235`)
- [x] `crate::arch::irq_disable` (callers: `IntMutex::attempt_acquire_lock`, `IntMutex::lock_block_yield`, `suspend_impl`; sites: `src/sync.rs:75`, `src/sync.rs:101`, `src/thread.rs:256`)
- [x] `crate::arch::save_context` (caller: `suspend_impl`; site: `src/thread.rs:267`)
- `crate::arch::sleep_core` (caller: `poll_tasks`; site: `src/thread.rs:237`)
- [x] `crate::arch::IrqState::save` (callers: `IntMutex::lock`, `suspend_impl`; sites: `src/sync.rs:126`, `src/thread.rs:255`)
- [x] `crate::arch::IrqState::restore` (callers: `IntMutexGuard::drop`, `IntMutex::attempt_acquire_lock`, `suspend_impl`; sites: `src/sync.rs:24`, `src/sync.rs:82`, `src/thread.rs:274`)
- [x] `crate::arch::Context::jump_to` (caller: `go_to_thread`; site: `src/thread.rs:196`)
- [x] `crate::arch::Context::setup_for_call` (caller: `spawn_thread`; site: `src/thread.rs:335`)
### cpu & thread local storage
- [x] `crate::arch::set_cpu_local_pointer`
- [x] `crate::arch::get_cpu_local_pointer` (caller: `CoreLocalStorageHandler::get_base`; site: `src/mp.rs:61`)
- [x] `crate::arch::get_thread_local_pointer` (caller: `ThreadLocalStorageHandler::get_base`; site: `src/thread.rs:90`)
- [x] `crate::arch::set_thread_local_pointer` (callers: `thread_enter`, `thread_exit`; sites: `src/thread.rs:171`, `src/thread.rs:177`)
### panicing & stack unwinding
- [x] `crate::arch::UnwindContext::get` (caller: `StackTrace::current`; site: `src/print.rs:302`)
- [x] `crate::arch::UnwindContext::valid` (caller: `impl Display for StackTrace::fmt`; site: `src/print.rs:311`)
- [x] `crate::arch::UnwindContext::return_address` (caller: `impl Display for StackTrace::fmt`; site: `src/print.rs:312`)
- [x] `crate::arch::UnwindContext::next` (caller: `impl Display for StackTrace::fmt`; site: `src/print.rs:315`)
- [x] `crate::arch::halt` (caller: `rust_panic_impl`; site: `src/main.rs:274`)

## arch trait
### notes
- I really want kernel main to initialze the cpu local table, it's not arch specific

### Arch global state
#### Core Local


### traits
```rust 
pub trait UnwindContext {
    /// Returns the current stack frame as an unwind context
    unsafe fn get() -> UnwindContext;
    pub unsafe fn valid(&self) -> bool {
        (unsafe { self.return_address() }) != 0
    }

    pub unsafe fn return_address(&self) -> u64 {
        unsafe { self.ptr.wrapping_add(1).read() }
    }
    
    pub fn from_ptr(ptr: *const u64) -> UnwindContext;

    pub unsafe fn next(&self) -> UnwindContext {
        Self::from_ptr(unsafe {self.ptr.read()} as *const u64)
    }
}
```
```rust
pub trait IrqState {
    // Save the current IrqState
    fn save() -> IrqState;
    fn is_masked() -> bool;
}

pub trait Context {
    type Arch: Arch<Context = Self>;
    /// from what i understand basically a constructor -- give your thread the correct perms
    fn setup_kthread_context(& mut self);
    fn jump_to(&self);
    fn setup_for_call(&mut self);
}

pub trait Arch {
    type Context: Context<Arch = Self>;
    /// returns true if this cpu is the bootstrap processor
    fn is_bsp(req: MPRequest, cpu: &Cpu) -> bool;
    /// calls initalize core
    fn initialize_mp(req: MPRequest) -> ! {
        let bsp = None;
        let mut core_id: u64 = 1;
        for cpu in req.cpus() {
            if Self::is_bsp(req, cpu) {
              bsp = Some(cpu);  
            } else {
                cpu.extra.store(core_id, Ordering::SeqCst);
                core_id+=1;
                cpu.goto_address.write(Self::start_core); 
            }
        }
        unsafe {Self::start_core(bsp.expect("Couldn't find the bootstrap processor"))}
    }
    /// does per core init
    /// this looks like:
    /// 1. setting up the cpu local ptr
    /// 2. setting up tables and interrupts
    /// 3. turning on needed features
    unsafe fn initialize_core(cpu: &Cpu) -> ();
    /// wrapper around initalize core that goes to kernel main
    unsafe extern "C" fn start_core(cpu: &Cpu) -> ! {
        unsafe {Self::initalize_core(cpu: &Cpu)};
        kernel_main()
    }
    unsafe fn set_irq_enabled(enabled: bool);
    unsafe fn restore(state: &IrqState) {
       set_irq_enabled(!state.is_masked()); 
    }
    /// save the current context and swith on to the provided temp stack & call fwd()
    unsafe fn save_context<T: FnOnce -> !>(
        temp_stack: &[u8],
        mut ctx: MutexGuard<'static, Context>,
        mut fwd: T
    );
    fn set_cpu_local_pointer(core_id: CoreId);
    fn get_cpu_local_pointer() -> u64;
    fn set_thread_local_pointer(base: *const u64);
    fn get_thread_local_pointer() -> u64;
    fn halt() -> !;  
}
```

## map of the kernel
### intialization
- on startup kernel calls `system_main` which does the following:
1. parses cmd line args (usually none)
2. sets up tty (via `init_tty` in the print module)
    - this function initalizes both the flanterm sink and the serial sink from the arch module. TODO: understand in detail the api between our kernel & flanterm
    (serial sink is via `arch::init_tty`)
3. initializes the heap. TODO: understand allocation
4. calls `initialize_mp` from the arch module
    - `initialize_mp`'s only job is to 1) initialize all cores 2) set up the cpu local table. 1) isn't arch specifc, it just relies on the number of cores. 
    - `initalize_mp` calls `initalize_core`. 2 jobs which must be done on core
        1. init cpu local ptr
        2. set up tables/interrupts (x86 actually has to then reset up the local ptr after setting up the tables, so can't _really_ abstract this away)
        3. (on aarch 64 we also have to turn on vector instructions, i think x86 gets around this by disabling them via a compiler flag, but we don't really have this luxury on aarch64, due to this the job of `initialize_core` is more like "set up each cpu")
    - `initalize_core` then calls kernel_main on each core
### cpu & thread local storage
- TODO: understand this in much more detail. tl;dr we copy local statics from the data section to the heap, once per core/thread
### threading
- `kernel_main`'s job is mainly to set up threading it does this via the following sequence of steps:
    - One core initalizes a barrier & theading (namely the work queue) for the system. All cores wait on this barrier
    - Once the barrier is passed, all cores set up their idle thread and wait on another barrier. After the final barrier is passed, each core turns on preemption and starts running the scheduler loop (via `poll_tasks`)
#### thread lifecycle
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
