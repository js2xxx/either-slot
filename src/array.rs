use alloc::vec::Vec;
use core::{
    array,
    iter::{self, FusedIterator, TrustedLen},
    marker::PhantomData,
    mem::MaybeUninit,
    ptr,
};

use crate::include::*;

const MAX_COUNT: usize = isize::MAX as _;

/// The storage of elements in the slot.
///
/// The user should only use this type when constructing the type of custom
/// storaging [`Place`]s. Only [`Default::default`] can be used to initialize
/// this type.
#[derive(Debug)]
pub struct Element<T> {
    storage: UnsafeCell<MaybeUninit<T>>,
    placed: AtomicBool,
}

impl<T> Default for Element<T> {
    fn default() -> Self {
        Element {
            storage: UnsafeCell::new(MaybeUninit::uninit()),
            placed: AtomicBool::new(false),
        }
    }
}

impl<T> Element<T> {
    pub fn vec(count: usize) -> Vec<Self> {
        iter::repeat_with(Default::default)
            .take(count)
            .collect::<Vec<_>>()
    }

    pub fn array<const N: usize>() -> [Self; N] {
        array::from_fn(|_| Default::default())
    }

    /// # Safety
    ///
    /// - This element slot must not hold a value when the function is called.
    /// - The caller must append a [`Release`] fence if atomic ordering is
    ///   desired.
    pub(crate) unsafe fn place(&self, data: T) {
        unsafe { self.storage.with_mut(|ptr| (*ptr).write(data)) };
        self.placed.store(true, Relaxed);
    }

    /// # Safety
    ///
    /// - This function must be called only once if this element slot holds a
    ///   value.
    /// - The caller must prepend an [`Acquire`] fence if atomic ordering is
    ///   desired.
    pub(crate) unsafe fn take(&self) -> Option<T> {
        self.placed
            .load(Relaxed)
            .then(|| unsafe { self.storage.with_mut(|ptr| (*ptr).assume_init_read()) })
    }
}

/// The custom storage place of [`Element`]s in the slot.
///
/// This trait should not be directly implemented; users should implement
/// [`AsRef`] to `[Element<T>]` instead. We don't make this trait an alias of
/// [`core::ops::Deref`] because arrays don't implement this trait.
pub trait Place<T>: AsRef<[Element<T>]> {}
impl<T, P> Place<T> for P where P: AsRef<[Element<T>]> {}

struct Inner<T, P>
where
    P: Place<T>,
{
    count: AtomicUsize,
    place: P,
    marker: PhantomData<[T]>,
}

impl<T, P> Inner<T, P>
where
    P: Place<T>,
{
    const LAYOUT: Layout = Layout::new::<Self>();

    fn new(place: P) -> NonNull<Self> {
        let count = place.as_ref().len();
        assert!(
            count <= MAX_COUNT,
            "the length of the slot must not exceed `isize::MAX`"
        );

        let memory = match Global.allocate(Self::LAYOUT) {
            Ok(memory) => memory.cast::<Self>(),
            Err(_) => handle_alloc_error(Self::LAYOUT),
        };
        let value = Self {
            count: AtomicUsize::new(count),
            place,
            marker: PhantomData,
        };
        // SAFETY: We own this fresh uninitialized memory whose layout is the same as
        // this type.
        unsafe { memory.as_ptr().write(value) }
        memory
    }

    /// # Safety
    ///
    /// 1. `this` must own a valid `Inner` uniquely (a.k.a. no other references
    ///    to the structure), and use an [`Acquire`] fence if atomic ordering is
    ///    desired.
    /// 2. `start` must be less than the length of `place` in `this`.
    /// 3. The caller must not use `this` again since it is consumed and dropped
    ///    in this function.
    unsafe fn drop_in_place(this: NonNull<Self>, start: usize) {
        // SAFETY: See contract 1.
        let inner = unsafe { this.as_ref() };
        // SAFETY: See contract 2.
        for elem in inner.place.as_ref().get_unchecked(start..) {
            // SAFETY: See contract 1.
            unsafe { drop(elem.take()) }
        }
        // SAFETY: See contract 3.
        unsafe { ptr::drop_in_place(this.as_ptr()) };
        // SAFETY: See contract 3.
        unsafe { Global.deallocate(this.cast(), Inner::<T, P>::LAYOUT) };
    }
}

/// The placer of an array slot.
///
/// The user can only access the slot once by this structure.
#[derive(Debug)]
pub struct Sender<T, P>
where
    P: Place<T>,
{
    inner: NonNull<Inner<T, P>>,
    index: usize,
}

// SAFETY: We satisfy the contract by exposing no reference to any associated
// function, and provide an atomic algorithm during its access or dropping
// process, which satisfies the need of `Send`.
unsafe impl<T: Send, P: Place<T>> Send for Sender<T, P> {}

impl<T, P> Sender<T, P>
where
    P: Place<T>,
{
    /// # Safety
    ///
    /// 1. `inner` must hold a valid immutable reference to `Inner`.
    /// 2. `start` must be less than the length of `place` in `inner`.
    unsafe fn new(inner: NonNull<Inner<T, P>>, index: usize) -> Self {
        Sender { inner, index }
    }

    /// Place the value into the slot, or obtain the resulting iterator if no
    /// other senders exist any longer.
    pub fn send(self, value: T) -> Result<(), SenderIter<T, P>> {
        // SAFETY: See contract 1 in `Self::new`.
        let inner = unsafe { self.inner.as_ref() };
        // SAFETY: See contract 2 in `Self::new`.
        let elem = unsafe { inner.place.as_ref().get_unchecked(self.index) };

        // SAFETY: Each sender has its ownership of one `Element` storage in its
        // `inner`, and thus the placing is safe. Besides, the appending `Release`
        // ordering is supplied.
        unsafe { elem.place(value) };
        let fetch_sub = inner.count.fetch_sub(1, Release);

        let pointer = self.inner;
        // We don't want to call the dropper anymore because it decreases the reference
        // count once more.
        mem::forget(self);

        if fetch_sub == 1 {
            // SAFETY: We use `Acquire` fence here to observe other executions of placing
            // values. And since the reference count is now 0, we owns `inner`, so it can be
            // handed to the iterator safely.
            atomic::fence(Acquire);
            return Err(unsafe { SenderIter::new(pointer) });
        }
        Ok(())
    }
}

impl<T, P: Place<T>> Drop for Sender<T, P> {
    fn drop(&mut self) {
        // SAFETY: See contract 1 in `Self::new`.
        let inner = unsafe { self.inner.as_ref() };
        // No additional ordering is used because we now have no more
        // observations/modifications to slot values, except...
        if inner.count.fetch_sub(1, Relaxed) == 1 {
            // SAFETY: ... we now owns our `inner`.
            atomic::fence(Acquire);
            unsafe { Inner::drop_in_place(self.inner, 0) }
        }
    }
}

/// The resulting iterator of values that all the senders have placed into the
/// slot.
///
/// Obtaining this structure means other senders all have been consumed or
/// dropped, which causes the inconsistency of the count of values yielded.
#[derive(Debug)]
pub struct SenderIter<T, P>
where
    P: Place<T>,
{
    inner: NonNull<Inner<T, P>>,
    index: usize,
}

// SAFETY: We now owns `inner`.
unsafe impl<T: Send, P: Place<T>> Send for SenderIter<T, P> {}

impl<T, P: Place<T>> SenderIter<T, P> {
    /// # Safety
    ///
    /// `inner` must owns a valid `Inner`.
    unsafe fn new(inner: NonNull<Inner<T, P>>) -> Self {
        Self { inner, index: 0 }
    }
}

impl<T, P: Place<T>> Iterator for SenderIter<T, P> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        // SAFETY: See contract 1 in `Sender::new`.
        let inner = unsafe { self.inner.as_ref() };

        // `index` in the iterator is not always less than its length, so we use the
        // safe `get` to access the element storage.
        while let Some(elem) = inner.place.as_ref().get(self.index) {
            self.index += 1;

            // SAFETY: We now owns `inner`, so no atomic ordering is needed; each element is
            // only taken once since `index` is incremented at every yield.
            if let Some(data) = unsafe { elem.take() } {
                return Some(data);
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        // SAFETY: See contract 1 in `Sender::new`.
        let inner = unsafe { self.inner.as_ref() };
        let len = inner.place.as_ref().len();
        (len, Some(len))
    }
}

impl<T, P: Place<T>> ExactSizeIterator for SenderIter<T, P> {}

impl<T, P: Place<T>> FusedIterator for SenderIter<T, P> {}

unsafe impl<T, P: Place<T>> TrustedLen for SenderIter<T, P> {}

impl<T, P: Place<T>> Drop for SenderIter<T, P> {
    fn drop(&mut self) {
        // SAFETY: We now owns `inner`, so no atomic ordering is needed; `index` is
        // always equal or less then the length of `place`.
        unsafe { Inner::drop_in_place(self.inner, self.index) }
    }
}

/// The initialization iterator for senders.
///
/// The senders are ALREADY initialized upon the construction of this iterator.
/// This structure is implemented to get rid of additional potential memory
/// allocations.
///
/// When the iterator is dropped, it will drop all the senders yet to be
/// yielded.
#[derive(Debug)]
pub struct InitIter<T, P: Place<T>> {
    inner: NonNull<Inner<T, P>>,
    index: usize,
}

unsafe impl<T: Send, P: Place<T>> Send for InitIter<T, P> {}

impl<T, P: Place<T>> InitIter<T, P> {
    /// # Safety
    ///
    /// `inner` must owns a valid `Inner`.
    unsafe fn new(inner: NonNull<Inner<T, P>>) -> Self {
        InitIter { inner, index: 0 }
    }
}

impl<T, P: Place<T>> Iterator for InitIter<T, P> {
    type Item = Sender<T, P>;

    fn next(&mut self) -> Option<Self::Item> {
        // SAFETY: See contract 1 in `Sender::new`.
        let inner = unsafe { self.inner.as_ref() };
        let len = inner.place.as_ref().len();
        if self.index < len {
            // SAFETY: `inner` is immutable; `index` is in (0..len).
            let s = unsafe { Sender::new(self.inner, self.index) };
            self.index += 1;
            Some(s)
        } else {
            None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        // SAFETY: See contract 1 in `Sender::new`.
        let inner = unsafe { self.inner.as_ref() };
        let len = inner.place.as_ref().len();
        (len, Some(len))
    }
}

impl<T, P: Place<T>> Drop for InitIter<T, P> {
    fn drop(&mut self) {
        self.for_each(drop)
    }
}

impl<T, P: Place<T>> ExactSizeIterator for InitIter<T, P> {}

impl<T, P: Place<T>> FusedIterator for InitIter<T, P> {}

unsafe impl<T, P: Place<T>> TrustedLen for InitIter<T, P> {}

/// Construct an iterator of senders to a slot, whose values will be placed on
/// `place`.
pub fn from_place<T, P: Place<T>>(place: P) -> InitIter<T, P> {
    let inner = Inner::new(place);
    // SAFETY: `inner` owns `Inner`.
    unsafe { InitIter::new(inner) }
}

/// Construct an iterator of senders to a slot, whose values will be placed on a
/// [`Vec`].
pub fn vec<T>(count: usize) -> InitIter<T, Vec<Element<T>>> {
    from_place(Element::vec(count))
}

/// Construct an array of senders to a slot, whose values will be placed on an
/// array.
///
/// This function is specialized to returning an array of senders instead of an
/// iterator in order to keep resulting length constant.
///
/// # Examples
///
/// ```rust
/// let [s1, s2, s3] = either_slot::array();
/// s1.send(1).unwrap();
/// s2.send(2).unwrap();
/// let iter = s3.send(3).unwrap_err();
/// assert_eq!(iter.collect::<Vec<_>>(), [1, 2, 3]);
/// ```
///
/// ```rust
/// let [s1, s2, s3] = either_slot::array();
/// drop(s1);
/// s3.send(3).unwrap();
/// let iter = s2.send(2).unwrap_err();
/// assert_eq!(iter.collect::<Vec<_>>(), [2, 3]);
/// ```
pub fn array<T, const N: usize>() -> [Sender<T, [Element<T>; N]>; N] {
    let inner = Inner::new(Element::array());
    // SAFETY: `inner` is immutable; index is in (0..N).
    array::from_fn(move |index| unsafe { Sender::new(inner, index) })
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;
    #[cfg(not(loom))]
    use std::thread;

    #[cfg(loom)]
    use loom::thread;

    use crate::array::{from_place, Element};

    #[test]
    fn send() {
        fn inner() {
            let j = from_place(Element::array::<3>())
                .enumerate()
                .map(|(i, s)| thread::spawn(move || s.send(i)))
                .collect::<Vec<_>>();

            let iter = j
                .into_iter()
                .map(|j| j.join().unwrap())
                .fold(Ok(()), Result::and)
                .unwrap_err();

            assert_eq!(iter.collect::<Vec<_>>(), [0, 1, 2]);
        }

        #[cfg(not(loom))]
        inner();
        #[cfg(loom)]
        loom::model(inner);
    }

    #[test]
    fn drop_one() {
        fn inner() {
            let j = from_place(Element::vec(3))
                .enumerate()
                .map(|(i, s)| {
                    if i != 1 {
                        thread::spawn(move || s.send(i))
                    } else {
                        thread::spawn(move || {
                            drop(s);
                            Ok(())
                        })
                    }
                })
                .collect::<Vec<_>>();

            let res = j
                .into_iter()
                .map(|j| j.join().unwrap())
                .fold(Ok(()), Result::and);

            if let Err(iter) = res {
                assert_eq!(iter.collect::<Vec<_>>(), [0, 2]);
            }
        }

        #[cfg(not(loom))]
        inner();
        #[cfg(loom)]
        loom::model(inner);
    }
}
