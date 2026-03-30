# State

There are various states of the system that periodically must be saved (interrupts, preemption, core pinning, etc.) and modified when performing a sensitive operation and restored when that operation is complete. We define a trait `StateTrait` that governs how this state is accessed and modified. From this, we automatically generate a struct `State` that stores a state of the given kind, with functions to save the current state into such a struct and restore it from such a struct. Lastly, many such states , so from a `State` we also generate a `StateGuard` that automatically restores the state stored within when it goes out of scope. This makes it easy to generate such guards: only the methods for interaction with the actual state of the system need to be implemented, and the rest is automatically generated. 

The `StateGuard` can be used in a `let _guard = StateGuard::<MyState>::guard();` statement to automatically save the current state, set the state to a "guarded" value, and restore the original state when `_guard` goes out of scope. Alternatively, `StateGuard::preserve()` can be used to save the current state without modifying it, and still restore it when the guard goes out of scope.

## StateTrait

The `StateTrait` defines the interface for accessing and modifying a particular kind of state.
```rust
pub trait StateTrait {
    type Value: Copy; // the type of value that represents this state
    fn get() -> Self::Value; // gets the current actual state of the thing
    fn set(val: Self::Value); // sets the current actual state of the thing
    fn exchange_guarded() -> Self::Value; // "locks down" the actual state
}
```
## State struct

The `State` struct is a simple wrapper around the underlying `StateTrait`, automatically generated thereof via generics. It provides methods to create a new `State` from a given value, to save the current state into a `State` object, and to restore the actual state from a `State` object.
```rust
#[derive(Clone, Copy)]
pub struct State<S: StateTrait>(S::Value);

impl<S: StateTrait> State<S> {
    pub fn new(val: S::Value) -> Self {
        // creates object to represent the current actual state
        Self(val)
    }
    pub fn save() -> Self {
        // creates object to represent the current actual state
        Self(S::get())
    }
    pub fn restore(&self) {
        // sets actual state of thing to this saved state
        S::set(self.0)
    }
}
```

## StateGuard
This struct provides a convenient RAII-style guard that automatically restores the state when it goes out of scope. It can be created in two ways: `guard()` which saves the current state and sets the actual state to a "guarded" value, and `preserve()` which saves the current state without modifying it. In both cases, when the `StateGuard` goes out of scope, it will automatically restore the original state.

```rust
pub struct StateGuard<S: StateTrait>(State<S>);

impl<S: StateTrait> Drop for StateGuard<S> {
    fn drop(&mut self) {
        self.0.restore();
    }
}

impl<S: StateTrait> StateGuard<S> {
    pub fn guard() -> Self {
        StateGuard(State::<S>(S::exchange_guarded()))
    }
    pub fn preserve() -> Self {
        StateGuard(State::<S>::save())
    }
```


