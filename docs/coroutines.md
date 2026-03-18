# Stackless Coroutines
- `kernel_main` also sets up coroutines, which sit on top of threads, via the following sequence of steps:
    - One core initializes the global coroutine ready queue before the barrier mentioned in [threading](#threading).
    - Once the barrier is passed, all cores set up a executor thread.

## Coroutine Lifecycle
- Coroutines are created via `spawn_coroutine` which takes a Future (e.g., output of an async fn call) and adds it to the coroutine ready queue.
- **Polling**:
    - The compiler does magic and generates code to allow Futures to work. Each one has a `poll` function to make some progress. See [this section](#how-do-coroutines-actually-work) for more details.
    - Each executor thread waits on the coroutine ready queue to poll Futures in a loop (with `yield_thread` after a batch number).
    - It is the responsibility of the Future to call its Waker when it is ready to make more progress, which adds it back into the coroutine ready queue.
        - You do not need to worry about this unless you are implementing your own Future, as correctly implemented Futures already do this.

## Using Coroutines

#### Using Others' Futures and/or async fns
- This is for if you are using a Future and/or `async fn` implemented by someone else.

- You can only use `async`/`await` in an `async fn`, so if you need the return value, you will need to write an `async fn` with no return value that calls the Future/`async fn` and does the work you want.
- Then, call `spawn_coroutine` on a call to the `async fn`.
    - This function exits immediately, and the coroutine will run in the background when scheduled.
```rust
// Note that the ellipses are not part of the syntax.
async fn do_work(/* ... */) { // No return value.
    // Call await to get the value.
    let result0 = some_async_fn(/* ... */).await;
    let result1 = some_future.await;
    // ...
}

// Somewhere else in the code in a non-async function.
spawn_coroutine(do_work(/* ... */));
```
- **If you want to wait for the result**, you will either need to pass in some synchronization primitive or wrap what you want to do with the result in the same or another coroutine.

#### Using Your Own Futures
- This is for if you are implementing your own Future.

- First, you will need to implement the Future trait, which is specifying the `Output` type and implementing the `poll` function.
```rust
pub trait Future {
    type Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output>;
}

enum Poll<T> {
    Ready(T),
    Pending,
}
```
- **Polling**:
    - This is where you will make progress. Note that `self` is `Pin<&mut Self>`, so you can mutate the Future's internal state (e.g, to remember how much progress you have made) and have self-referential structures (e.g., a pointer to yourself or an offset from your start).
    - If you are finished and have a value, return `Poll::Ready(T)` with the value.
    - If you are not finished, return `Poll::Pending`, and make sure you <ins>**call the Waker**</ins> when you are ready to make more progress. This can be done by saving the Waker somewhere.
        - If you don't call the Waker, you will never be polled again, thus never making progress.
- Note that there are two functions to call the Waker.
```rust
pub trait Waker {
    fn wake(self: Arc<Self>); // This consumes the Waker, so you can only call it once.
    fn wake_by_ref(self: &Arc<Self>); // This does not consume the Waker.
}
```
- Example Future implementations:
```rust
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};

struct CustomFuture0 {
    poll_count: u64, // You will likely want to store some kind of state.
}

impl Future for CustomFuture0 {
    type Output = u64; // Output type goes here.

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        this.poll_count += 1; // Make some progress.

        if this.poll_count >= 1624252 {
            Poll::Ready(this.poll_count) // Finished.
        } else {
            cx.waker().wake_by_ref(); // Call the Waker to be polled again.
            Poll::Pending // Not finished yet.
        }
    }
}
```
```rust
struct CustomFuture1 {
    poll_count: u64,
    waker: Option<Waker>, // Or, you can store the Waker somewhere to be called elsewhere.
}

impl Future for CustomFuture1 {
    type Output = u64;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        this.poll_count += 1; // Make some progress.

        if this.poll_count >= 1624252 {
            Poll::Ready(this.poll_count)
        } else {
            this.waker = Some(cx.waker().clone()); // Save the Waker to be called later.
            // You should replace the saved Waker with the new Waker in case it is different.
            // You can also check waker.will_wake to see if both Wakers wake the same task,
            // though cloning Wakers is already supposedly cheap.
            Poll::Pending
        }
    }
}

// Somewhere else in the code, you should call waker.wake().
```
- You are guaranteed to be polled again after calling the Waker.
- Now, you can use your future like any other Future.
```rust
// You can also create an async fn with the result as a return value.
async fn get_1624252() -> u64 {
    // Theoretically you have more than just this line.
    CustomFuture0 { poll_count: 0 }.await
}

async fn print_results() {
    let result0 = get_1624252().await;
    let result1 = CustomFuture1 { poll_count: 0, waker: None }.await;
    kprintln!("Results: {}, {}", result0, result1);
}

// Somewhere else in the code in a non-async function.
spawn_coroutine(print_results());
```
- Of course, you can also have a Future with no return value that just does work during polling.
```rust
spawn_coroutine(FutureThatDoesWork { /* ... */ });
```

## How Do Coroutines Actually Work?
- This is pretty similar to how they work in general, but we will be using Rust-specific syntax with `async`/`await`. Most of the traits/enums have already been mentioned [above](#using-coroutines) and is thus not repeated here.

- The compiler generates code to convert the existing code to a state machine, allowing a coroutine to suspend execution (pause) at certain points (e.g., at an `await` or the end of a `poll` if it is still pending) and resume later.
```rust
async fn read_and_add() -> u64 {
    kprintln!("Beginning.");
    let first = some_async_read64_function().await;
    kprintln!("Middle.");
    let second = some_async_read64_function().await;
    kprintln!("End.");
    first + second
}
```
- The return value of this is really `Future<Output = u64>`.
- In this example, the compiler will essentially split the function into 3 parts separated by the `await`s (or more depending on how `some_async_read64_function` works). It also stores an internal state to keep track of where we are and know which part to resume.

- **Stackless**:
    - The compiler knows the exact amount of memory needed for the rest of the execution, so it can allocate a fixed-size section of memory somewhere like the heap in place of the stack, thus allowing the coroutine to be "stackless" (though this fixed-size section is essentially its stack but just fitted to exactly the amount needed).
- **Polling**:
    - As stated above, this is a function of the Future trait that lets the coroutine make some progress. This is where the coroutine can resume execution, returning `Poll::Pending` or `Poll::Ready(T)` (with the ready value).
- **Wakers**:
    - The Future calls its Waker when it is ready to make more progress. This lets the executor know the coroutine is ready to be polled again. In our implementation, this means adding the coroutine back into the ready queue.
