pub use alloc::alloc::{handle_alloc_error, Global};
pub use core::{
    alloc::{Allocator, Layout},
    hint,
    mem::{self, ManuallyDrop},
    ptr::NonNull,
    sync::atomic::{AtomicU8, Ordering::*},
};

#[derive(Debug)]
pub(crate) struct UnsafeCell<T>(core::cell::UnsafeCell<T>);

impl<T> UnsafeCell<T> {
    pub(crate) fn new(data: T) -> UnsafeCell<T> {
        UnsafeCell(core::cell::UnsafeCell::new(data))
    }

    pub(crate) fn with_mut<R>(&self, f: impl FnOnce(*mut T) -> R) -> R {
        f(self.0.get())
    }
}
