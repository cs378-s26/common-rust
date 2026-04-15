use core::sync::atomic::Ordering;

use crate::{
    Arch, ArchTrait,
    thread::{CAN_YIELD, PINNED_TO_CORE},
};

// for saving and restoring interrupts, preemption, core pinning, ...
pub trait StateTrait {
    type Value: Copy; // the type of value that represents this state
    fn get() -> Self::Value; // gets the current actual state of the thing
    fn set(val: Self::Value); // sets the current actual state of the thing
    fn exchange_guarded() -> Self::Value; // "locks down" the actual state
}

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
}

#[derive(Clone, Copy)]
pub struct Irq;

impl StateTrait for Irq {
    type Value = bool;
    #[inline(always)]
    fn set(val: Self::Value) {
        Arch::set_irq_enabled(val);
    }
    #[inline(always)]
    fn get() -> Self::Value {
        Arch::irq_is_enabled()
    }
    #[inline(always)]
    fn exchange_guarded() -> Self::Value {
        let val = Arch::irq_is_enabled();
        Arch::set_irq_enabled(false);
        val
    }
}

#[derive(Clone, Copy)]
pub struct CorePin;

impl StateTrait for CorePin {
    type Value = bool;
    #[inline(always)]
    fn set(val: bool) {
        PINNED_TO_CORE.store(val, Ordering::Release);
    }
    #[inline(always)]
    fn get() -> Self::Value {
        PINNED_TO_CORE.load(Ordering::Acquire)
    }
    #[inline(always)]
    fn exchange_guarded() -> Self::Value {
        PINNED_TO_CORE.swap(true, Ordering::AcqRel)
    }
}

#[derive(Clone, Copy)]
pub struct Preemption;
impl StateTrait for Preemption {
    type Value = bool;
    #[inline(always)]
    fn set(val: bool) {
        CAN_YIELD.store(val, Ordering::Release);
    }
    #[inline(always)]
    fn get() -> Self::Value {
        CAN_YIELD.load(Ordering::Acquire)
    }
    #[inline(always)]
    fn exchange_guarded() -> Self::Value {
        CAN_YIELD.swap(false, Ordering::AcqRel)
    }
}
