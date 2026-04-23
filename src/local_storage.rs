use core::{ffi::c_void, slice};

extern crate alloc;

use alloc::boxed::Box;

use crate::memory::heap::aligned_slice;

pub trait LocalStorageHandler {
    fn get_range() -> (*const c_void, *const c_void);

    fn get_template() -> &'static [u8] {
        let (start, end) = Self::get_range();
        let start = start as *const u8;
        let end = end as *const u8;

        let len = unsafe { end.offset_from(start) } as usize;
        unsafe { slice::from_raw_parts(start, len) }
    }

    fn create() -> Box<[u8]> {
        let template = Self::get_template();
        let len = template.len();
        let mut out = aligned_slice(len, 16);
        out.copy_from_slice(template);
        out
    }

    fn get_base() -> u64;
}

pub trait LocalStorage<T> {
    type Handler: LocalStorageHandler;

    fn offset(&self) -> u64 {
        // WTF
        (self as *const _ as *const () as u64) - Self::Handler::get_range().0 as u64
    }

    fn get_inst(&self) -> &T {
        let addr = Self::Handler::get_base() + self.offset();
        unsafe { &*(addr as *const T) }
    }
}

pub macro impl_local_storage($type:tt, $handler:ty) {
    impl<T> LocalStorage<T> for $type<T> {
        type Handler = $handler;
    }

    unsafe impl<T> Send for $type<T> {}
    unsafe impl<T> Sync for $type<T> {}

    impl<T> core::ops::Deref for $type<T> {
        type Target = T;

        fn deref(&self) -> &Self::Target {
            self.get_inst()
        }
    }
}
