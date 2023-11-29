#![doc = include_str!("../README.md")]
#![no_std]
#![feature(allocator_api)]
#![feature(trusted_len)]
#![cfg_attr(loom, feature(alloc_layout_extra))]
#![cfg_attr(test, feature(assert_matches))]

#[cfg_attr(not(loom), path = "include_core.rs")]
#[cfg_attr(loom, path = "include_loom.rs")]
mod include;

pub mod array;
pub mod tuple;

use self::include::*;
pub use self::{
    array::{array, vec},
    tuple::tuple,
};

extern crate alloc;

#[cfg(test)]
extern crate std;

union Place<A, B> {
    uninit: (),
    a: ManuallyDrop<A>,
    b: ManuallyDrop<B>,
}

const INIT: u8 = 0;
const WRITING: u8 = 1;
const HAS_A: u8 = 2;
const HAS_B: u8 = 3;
const DONE: u8 = 4;

struct Inner<A, B> {
    state: AtomicU8,
    place: UnsafeCell<Place<A, B>>,
}

impl<A, B> Inner<A, B> {
    const LAYOUT: Layout = Layout::new::<Self>();

    fn new() -> NonNull<Self> {
        let memory = match Global.allocate(Self::LAYOUT) {
            Ok(memory) => memory.cast::<Self>(),
            Err(_) => handle_alloc_error(Self::LAYOUT),
        };
        let value = Self {
            state: AtomicU8::new(INIT),
            place: UnsafeCell::new(Place { uninit: () }),
        };
        unsafe { memory.as_ptr().write(value) }
        memory
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SendError<P, Q> {
    Received(P, Q),
    Disconnected(P),
}

#[derive(Debug)]
pub struct ASender<A, B>(NonNull<Inner<A, B>>);

#[derive(Debug)]
pub struct BSender<A, B>(NonNull<Inner<A, B>>);

unsafe impl<A: Send, B: Send> Send for ASender<A, B> {}
unsafe impl<A: Send, B: Send> Send for BSender<A, B> {}

impl<A, B> ASender<A, B> {
    const LAYOUT: Layout = Inner::<A, B>::LAYOUT;

    pub fn send(self, a: A) -> Result<(), SendError<A, B>> {
        let inner = unsafe { self.0.as_ref() };
        loop {
            match inner
                .state
                .compare_exchange(INIT, WRITING, Acquire, Acquire)
            {
                Ok(_) => {
                    let a = ManuallyDrop::new(a);
                    unsafe { inner.place.with_mut(|ptr| ptr.write(Place { a })) };
                    inner.state.store(HAS_A, Release);

                    mem::forget(self);
                    break Ok(());
                }
                Err(state) => match state {
                    WRITING => hint::spin_loop(),
                    HAS_B => {
                        let b = unsafe { inner.place.with_mut(|ptr| ptr.read().b) };
                        inner.state.store(DONE, Release);

                        break Err(SendError::Received(a, ManuallyDrop::into_inner(b)));
                    }
                    DONE => break Err(SendError::Disconnected(a)),
                    _ => unreachable!(),
                },
            }
        }
    }
}

impl<A, B> Drop for ASender<A, B> {
    fn drop(&mut self) {
        let inner = unsafe { self.0.as_ref() };
        loop {
            let state = inner.state.load(Acquire);
            if state != WRITING {
                match state {
                    INIT => {
                        if inner
                            .state
                            .compare_exchange_weak(INIT, DONE, AcqRel, Acquire)
                            .is_ok()
                        {
                            break;
                        }
                    }
                    HAS_B => inner
                        .place
                        .with_mut(|ptr| unsafe { ManuallyDrop::drop(&mut (*ptr).b) }),
                    DONE => {}
                    _ => unreachable!(),
                }
                unsafe { Global.deallocate(self.0.cast(), Self::LAYOUT) };
                break;
            }
            hint::spin_loop();
        }
    }
}

impl<A, B> BSender<A, B> {
    const LAYOUT: Layout = Inner::<A, B>::LAYOUT;

    pub fn send(self, b: B) -> Result<(), SendError<B, A>> {
        let inner = unsafe { self.0.as_ref() };
        loop {
            match inner
                .state
                .compare_exchange(INIT, WRITING, Acquire, Acquire)
            {
                Ok(_) => {
                    let b = ManuallyDrop::new(b);
                    unsafe { inner.place.with_mut(|ptr| ptr.write(Place { b })) };
                    inner.state.store(HAS_B, Release);

                    mem::forget(self);
                    break Ok(());
                }
                Err(state) => match state {
                    WRITING => hint::spin_loop(),
                    HAS_A => {
                        let a = unsafe { inner.place.with_mut(|ptr| ptr.read().a) };
                        inner.state.store(DONE, Release);

                        break Err(SendError::Received(b, ManuallyDrop::into_inner(a)));
                    }
                    DONE => break Err(SendError::Disconnected(b)),
                    _ => unreachable!(),
                },
            }
        }
    }
}

impl<A, B> Drop for BSender<A, B> {
    fn drop(&mut self) {
        let inner = unsafe { self.0.as_ref() };
        loop {
            let state = inner.state.load(Acquire);
            if state != WRITING {
                match state {
                    INIT => {
                        if inner
                            .state
                            .compare_exchange_weak(INIT, DONE, AcqRel, Acquire)
                            .is_ok()
                        {
                            break;
                        }
                    }
                    HAS_A => inner
                        .place
                        .with_mut(|ptr| unsafe { ManuallyDrop::drop(&mut (*ptr).a) }),
                    DONE => {}
                    _ => unreachable!(),
                }
                unsafe { Global.deallocate(self.0.cast(), Self::LAYOUT) };
                break;
            }
            hint::spin_loop();
        }
    }
}

pub fn either<A, B>() -> (ASender<A, B>, BSender<A, B>) {
    let inner = Inner::new();
    (ASender(inner), BSender(inner))
}

#[cfg(test)]
mod tests {
    use std::assert_matches::assert_matches;
    #[cfg(not(loom))]
    use std::thread;

    #[cfg(loom)]
    use loom::thread;

    use crate::{either, SendError};

    #[cfg(not(loom))]
    #[test]
    fn basic() {
        let (a, b) = either();
        a.send(1).unwrap();
        assert_eq!(b.send('x'), Err(crate::SendError::Received('x', 1)));

        let (a, b) = either::<_, ()>();
        drop(b);
        assert_eq!(a.send(1), Err(SendError::Disconnected(1)));

        let _ = either::<i32, u8>();
    }

    #[test]
    fn send() {
        fn inner() {
            let (a, b) = either();
            let t = thread::spawn(move || a.send(1));
            let r1 = b.send('x');
            let r2 = t.join().unwrap();
            assert_matches!(
                (r1, r2),
                (Ok(()), Err(SendError::Received(1, 'x')))
                    | (Err(SendError::Received('x', 1)), Ok(()))
            )
        }
        #[cfg(not(loom))]
        inner();
        #[cfg(loom)]
        loom::model(|| inner());
    }

    #[test]
    fn drop_either() {
        fn inner() {
            let (a, b) = either::<i32, _>();
            let t = thread::spawn(move || drop(a));
            assert_matches!(b.send(1), Err(SendError::Disconnected(1)) | Ok(()));
            t.join().unwrap();
        }
        #[cfg(not(loom))]
        inner();
        #[cfg(loom)]
        loom::model(|| inner());
    }

    #[test]
    fn drop_both() {
        fn inner() {
            let (a, b) = either::<i32, u8>();
            let t = thread::spawn(move || drop(a));
            drop(b);
            t.join().unwrap();
        }
        #[cfg(not(loom))]
        inner();
        #[cfg(loom)]
        loom::model(|| inner());
    }
}
