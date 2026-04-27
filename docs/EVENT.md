# Kernel Events

This subsystem allows the kernel to trigger events that can be handled asynchronously on specified cores, which is necessary for implementing TLB shootdowns and other inter-processor communication. 

## Interface

There is an enum `Event` that specifies the type of event being triggered. To implement a new event, add a variant to the enum containing all the necessary data (including synchronization mechanisms), as well as a `match` case in the event handler specifying how to handle that event. 

To trigger an event, call `push_event(event: Event, core: CoreId, should_alloc: bool)`, which will push the event onto the specified core's event queue. Important: See [Design](#Design) for a discussion of when to use `should_alloc`.

## Design

Currently, during system initialization we create one event handler thread per core (pinned to that core, necessary to enable precise handling of TLB shootdowns) that continuously pops events from that core's event queue and handles them via a large `match` statement, yielding when it runs out of work to avoid busy-waiting or chewing up its quantum. Each core has a pre-allocated event queue (intrusive linked list) protected by a spinlock.

If `should_alloc` is true, we allocate a new event object on the heap to store the event information; otherwise, we use a pre-allocated event object pool that is reused for all events of that type on that core. The former is more expensive but allows for essentially unbounded concurrent events of the same type, while the latter is more efficient but can lead to lost events if multiple events of the same type are triggered on the same core before it has a chance to handle them.


