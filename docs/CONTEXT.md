# Context Switching 

Context switching is handled via the architecture specific `Context` struct. This struct holds the entire state of a thread, such as GP registers,
the page table pointer (`cr3` on x86), floating point/vector registers (if enabled), and all relevant CPU state. This includes registers that 
handle permission, which means context return logic must consider this.

Context switching, roughly speaking, is done in two halves. Some code is responsible for saving the context, and the `Context::jump_to(&self) -> !`
function will restore the cpu state to a context.

All context switches need to go through a "limbo" state on a separate stack where interrupts are masked. Here, preemption and yielding is disabled.
Abstractly speaking, all interrupt handlers are also in this "limbo" state, because we always switch stacks (on x86, this is done via the Interrupt
Stack Table). Some work can be done in this limbo state, because this is not *strictly* on any particular thread. For instance, all cooperative 
threading code will switch to a core-local stack before queuing the "previous" task into the relevant queue, as well as dropping the relevant 
spinlocks. 

Preemption is implemented by always scheduling to idle, which then queues the next task. Technically, this could be a handler thread. Some care needs 
to be taken in thinking about the edgecase where the preempted thread is idle itself. This should work, but there might be weird behavior when it 
comes to undefined behavior (or deadlock) in regards to pointer aliasing of the thread_context data structures. However, the "core" return 
infrastructure should not change, because at interrupt entry, we save the state of the current thread to its context.

## Limbo State 

The limbo state, which can also be referred to as the *between-stack state*, is a special state, defined by the following:
- The execution stack is never the stack of any thread in the kernel (possible stack states are the core-local `CTX_SWITCH_STACK`, an IST entry 
  for IRQ handlers, etc). 
- Interrupts must be disabled (no nested interrupts are supported - interrupts must be disabled on IRQ handler entry). On x86, this means masking `IF`.
- Interrupt handlers must be incredibly fast as a result, `<1000` cycles ideally. 
- No active thread `(CURRENT_THREAD == None)`, `is_on_thread()` is false.
- No `CTX_GUARD`
- No preemption

In this state, you must not:
- Block
- Allocate
- Take sleeping locks
- Attempt to schedule

NMI can happen, but they must do so via a custom stack and are required to never truly switch context. Such handlers must be perfectly transparent, 
with no modification of the CPU state (from the perspective of the interrupted code). NMIs must also consider that they may come from a non-thread 
context.

## Saving Context (first half of context switch)

Saving context happens in a variety of architecture specific functions. There are functions for saving context on cooperative yield, on irq entry, 
etc. The general rule of thumb is that the first thing done before/during the limbo state is to save the context of the "previous" state and then 
release all relevant locks, allowing it to be queued (not necessarily *scheduled*) again.

## Switching to Context (second half of context switch)

The function that handles switching to a context is `Context::jump_to(&self) -> !`. This function never returns (from the perspective of rust, at 
least). Interrupts must be disabled before calling this function. The caller must not be in a thread context, but rather a between-stack state. This 
can be checked with `is_on_thread()` (namely, it should be *false*). 

## Core Locals 

Some core locals are used when context switching.

- `CTX_GUARD` shall be `Some` when the core is on a thread. It is a lock guard that holds onto the context of the thread it is currently running.
- `CURRENT_THREAD` shall be some if `is_on_thread()`.

## Architecture Specifics 

On x86, interrupts are masked whenever an irq is entered by making all gates an interrupt gate (even for exceptions).

When returning from context, we use `iretq` to handle CS/SS descriptor permission level switches. Privilege transitions require correct handling of 
CS, SS, and potentially GS base (swapgs). Failure to restore segment state correctly may result in privilege escalation or memory corruption.
