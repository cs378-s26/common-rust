#![no_std]
#![feature(decl_macro)]
#![feature(const_trait_impl)]
#![feature(const_default)]
#![feature(slice_ptr_get)]
#![feature(box_as_ptr)]
#![feature(const_range)]
#![feature(never_type)]
#![feature(sync_unsafe_cell)]

pub mod arch;
pub mod coroutine;
pub mod cmdline;
pub mod heap;
pub mod mp;
pub mod print;
pub mod thread;
pub mod sync;
pub mod local_storage;
pub mod kern_main;

extern crate alloc;

