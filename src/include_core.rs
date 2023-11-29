pub use alloc::alloc::{handle_alloc_error, Global};
pub use core::{
    alloc::{Allocator, Layout},
    hint,
    mem::{self, ManuallyDrop},
    ptr::NonNull,
    sync::atomic::{self, AtomicBool, AtomicU8, AtomicUsize, Ordering::*},
};

#[derive(Debug)]
pub(crate) struct UnsafeCell<T: ?Sized>(core::cell::UnsafeCell<T>);

impl<T> UnsafeCell<T> {
    pub(crate) const fn new(data: T) -> UnsafeCell<T> {
        UnsafeCell(core::cell::UnsafeCell::new(data))
    }

    pub(crate) fn with_mut<R>(&self, f: impl FnOnce(*mut T) -> R) -> R {
        f(self.0.get())
    }
}
