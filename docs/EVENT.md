# Kernel Events

This subsystem, implemented in `event.rs`, allows other kernel components to trigger events then that can be handled asynchronously on specified cores.
There are two primary purposes of the events system: to force other cores to do things (in combination with IPIs), and to handle exceptions.

## Interface

There is an enum `Event` that specifies the type of event being triggered. To implement a new event, add a variant to the enum containing all the necessary data (including synchronization mechanisms), as well as a `match` case in the event handler specifying how to handle that event. 
Creating a new `Event` is strongly discouraged. This is messy work that modifies code at the core of our kernel. There are two things
the events system is meant to do: force other cores to do things, and to handle exceptions. If you are not doing either of those things, then you
should not be using the events system. If you are handling exceptions, I strongly encourage you to consider whether your needs can be better addressed
by implementing signals (and punting exception-handling logic to the user) rather than implementing it in the kernel.  

To trigger an event, call `push_event(event: Event, core: CoreId, should_alloc: bool)`, which will push the event onto the specified core's event queue. See [Allocation](#Allocation) for a discussion of when to use `should_alloc`.

## Design

During system initialization, we create one even handler thread per core (pinned to that core). Each core has a pre-allocated event queue, and an event thread queue (which I will henceforth
rather to as the thread queue). The event handler continuously consumes events from both queues, prioritizing the event queue over the thread queue (this is important for things like
TLB shootdowns). Both queues are represented as intrusive linked lists. 

### Allocation

If `should_alloc` is true, we allocate a new event object on the heap to store the event information; otherwise, we use a pre-allocated event object that is reused for all events on that core. The former is more expensive but allows for essentially unbounded concurrent events of the same type, while the latter is more efficient but can lead to lost events if multiple events of the same type are triggered on the same core before it has a chance to handle them.
