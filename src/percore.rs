extern crate alloc;

use core::{alloc::Layout, cell::Cell, ffi::c_void, ops::Deref, ptr::copy_nonoverlapping};

use alloc::vec::Vec;
use derive_more::{Debug, Display};
use spin::Once;

use crate::arch::get_cpu_local_pointer;

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
            $vis static $name: crate::percore::CoreLocal<$ty> = crate::percore::CoreLocal::new($init);
        )*
    }
}

unsafe extern "C" {
    static _marker_cpu_local_template_start: c_void;
    static _marker_cpu_local_template_end: c_void;
}

static OFFSET_ARRAY: Once<Vec<u64>> = Once::new();

fn cpu_local_template_region() -> (u64, u64) {
    (
        &raw const _marker_cpu_local_template_start as u64,
        &raw const _marker_cpu_local_template_end as u64,
    )
}

impl<T> CoreLocal<T> {
    pub const fn new(val: T) -> Self {
        Self(val)
    }

    fn offset(&self) -> u64 {
        let self_addr = self as *const _ as u64;
        let template_range = cpu_local_template_region();
        assert!((template_range.0..template_range.1).contains(&self_addr));
        self_addr - template_range.0
    }

    pub fn addr(&self) -> u64 {
        get_cpu_local_pointer() + self.offset()
    }
}

// core locals can always be "sent" and "synced" across threads (which is meaningless)
unsafe impl<T> Send for CoreLocal<T> {}
unsafe impl<T> Sync for CoreLocal<T> {}

impl<T> Deref for CoreLocal<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*(self.addr() as *const T) }
    }
}

pub fn get_cpu_local_pointer_for(core: CoreId) -> u64 {
    &raw const OFFSET_ARRAY.get().unwrap()[core.0] as u64
}

pub fn init_cpu_local_table(n_cores: usize) {
    let template = cpu_local_template_region();
    let len = (template.1 - template.0) as usize;

    OFFSET_ARRAY.call_once(|| {
        (0..n_cores)
            .map(|_| unsafe {
                let ptr = alloc::alloc::alloc(Layout::from_size_align(len, 16).unwrap());
                copy_nonoverlapping(template.0 as *const u8, ptr, len);
                ptr as u64
            })
            .collect()
    });
}

core_local! {
    pub CORE_ID: Cell<CoreId> = Cell::new(CoreId(0));
}
