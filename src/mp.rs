extern crate alloc;

use core::{cell::Cell, ffi::c_void};

use alloc::vec::Vec;
use atomic_enum::atomic_enum;
use derive_more::{Debug, Display};
use spin::Once;

use alloc::boxed::Box;

use crate::{
    arch::{Arch, ArchTrait},
    local_storage::{LocalStorageHandler, impl_local_storage},
};

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Display)]
#[display("{_0}")]
#[debug("CoreId({_0})")]
pub struct CoreId(pub usize);

// core local stuff

#[repr(C)]
pub struct CoreLocal<T>(T);

pub macro core_local {
    {
        $(
            $(#[$meta:meta])*
            $vis:vis $name:ident : $ty:ty = $init:expr;
        )*
    } => {
        $(
            $(#[$meta])*
            #[unsafe(link_section = ".cpu_local")]
            $vis static $name: crate::mp::CoreLocal<$ty> = crate::mp::CoreLocal::new($init);
        )*
    }
}

unsafe extern "C" {
    static _marker_cpu_local_template_start: c_void;
    static _marker_cpu_local_template_end: c_void;
}

pub struct CoreLocalStorageHandler;

impl LocalStorageHandler for CoreLocalStorageHandler {
    fn get_range() -> (*const c_void, *const c_void) {
        unsafe {
            (
                &_marker_cpu_local_template_start,
                &_marker_cpu_local_template_end,
            )
        }
    }

    fn get_base() -> u64 {
        Arch::get_cpu_local_pointer()
    }
}

impl<T> CoreLocal<T> {
    pub const fn new(val: T) -> Self {
        Self(val)
    }
}

impl_local_storage!(CoreLocal, CoreLocalStorageHandler);

// offset storage

static OFFSET_ARRAY: Once<Vec<u64>> = Once::new();

pub fn get_cpu_local_pointer_for(core: CoreId) -> u64 {
    &raw const OFFSET_ARRAY.get().unwrap()[core.0] as u64
}

pub fn init_cpu_local_table(n_cores: usize) {
    OFFSET_ARRAY.call_once(|| {
        (0..n_cores)
            .map(|_| Box::leak(CoreLocalStorageHandler::create()).as_ptr() as u64)
            .collect()
    });
}

core_local! {
    pub CORE_ID: Cell<CoreId> = Cell::new(CoreId(0));
}

impl<T: Send + Sync> CoreLocal<T> {
    pub fn read_for(&self, core: CoreId) -> &T {
        unsafe { &*(get_cpu_local_pointer_for(core) as *const T) }
    }
}

#[derive(PartialEq, Eq)]
#[atomic_enum]
pub enum MPStage {
    KInit,
    MPPreempt,
}

pub static MP_STAGE: AtomicMPStage = AtomicMPStage::new(MPStage::KInit);
