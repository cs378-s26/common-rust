# Various Notes on Ricky's Kernel

very confusing to jump into, hope this helps.


## overview of how the kenrel interacts with hardware
- on startup kernel calls `system_main` which calls `initialize_mp` from the arch module
    - `initialize_mp`'s only job is to 1) initialize all cores 2) set up the cpu local table. 1) isn't aarch specifc, it just relies on the number of cores. 
    - TODO: in the trait, have the method take an MpRequest that will be used to start the cores, & have system main setup the cpu local table
- `initalize_mp` calls `initalize_core`. 2 jobs which must be done on core
    1. init cpu local ptr
    2. set up tables/interrupts
- TODO: how can we implement the `init_core` interface while abstracting as much as we can? 


## threads & multiprocessing
After some architecture specific initialization, each core exectues `kernel_main`. There's a bit of bullshit but the gist of it is:
1. each thread sets up their idle thread & the current thread is set to it (more on this later)
2. preemption is enabled -- this currently doesn't do shit but it's a start

### threading model
currently we're in a cooperative threading model. the kernel only receives control when a thread calls yeild
