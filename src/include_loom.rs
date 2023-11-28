pub use core::{
    alloc::{AllocError, Allocator},
    mem::{self, ManuallyDrop},
    ptr::NonNull,
};

pub use ::alloc::alloc::handle_alloc_error;
pub use loom::{
    alloc::{alloc, dealloc, Layout},
    cell::UnsafeCell,
    hint,
    sync::atomic::{AtomicU8, Ordering::*},
};

pub struct Global;

unsafe impl Allocator for Global {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        match layout.size() {
            0 => Ok(NonNull::slice_from_raw_parts(layout.dangling(), 0)),
            // SAFETY: `layout` is non-zero in size,
            size => unsafe {
                let raw_ptr = alloc(layout);
                let ptr = NonNull::new(raw_ptr).ok_or(AllocError)?;
                Ok(NonNull::slice_from_raw_parts(ptr, size))
            },
        }
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        if layout.size() != 0 {
            // SAFETY: `layout` is non-zero in size,
            // other conditions must be upheld by the caller
            unsafe { dealloc(ptr.as_ptr(), layout) }
        }
    }
}
