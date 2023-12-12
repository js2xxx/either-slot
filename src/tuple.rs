mod utils;

use tuple_list::{Tuple, TupleList};

pub use self::utils::{Concat, Construct, InElement};
use self::utils::{Count, Index};
use crate::{array::Element, include::*};

#[derive(Debug)]
struct Inner<T: InElement> {
    count: AtomicUsize,
    place: T::Place,
}

impl<T: InElement> Inner<T> {
    const LAYOUT: Layout = Layout::new::<Self>();

    fn new() -> NonNull<Self> {
        let memory = match Global.allocate(Self::LAYOUT) {
            Ok(memory) => memory.cast::<Self>(),
            Err(_) => handle_alloc_error(Self::LAYOUT),
        };
        let value = Self {
            count: AtomicUsize::new(T::TUPLE_LIST_SIZE),
            place: T::init(),
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
    /// 2. The caller must not use `this` again since it is consumed and dropped
    ///    in this function.
    /// 3. `TAKEN` must be corresponding to the possible previous operation of
    ///    taking out all the data in the tuple.
    unsafe fn drop_in_place(this: NonNull<Self>) -> <T::Take as TupleList>::Tuple {
        // SAFETY: See contract 1.
        let inner = unsafe { this.as_ref() };

        let tuple = unsafe { T::take(&inner.place) }.into_tuple();

        // SAFETY: See contract 2.
        unsafe { Global.deallocate(this.cast(), Self::LAYOUT) };

        tuple
    }
}

type Whole<Head, Current, Tail> = <<Head as Concat<(Current,)>>::Output as Concat<Tail>>::Output;
type List<Head, Current, Tail> = <Whole<Head, Current, Tail> as Tuple>::TupleList;
type Ptr<Head, Current, Tail> = NonNull<Inner<List<Head, Current, Tail>>>;
type Place<Head, Current, Tail> = <List<Head, Current, Tail> as InElement>::Place;
type TakeList<Head, Current, Tail> = <List<Head, Current, Tail> as InElement>::Take;
type Take<Head, Current, Tail> = <TakeList<Head, Current, Tail> as TupleList>::Tuple;

/// The placer of an tuple slot.
///
/// The 3 generic represents the position of the target element of the tuple
/// this sender attempts to place. For example, `Sender<(A, B), C, (D, E, F)>`
/// represents a sender of `C` in `(A, B, C, D, E, F)`.
///
/// The user can only access the slot once by this structure.
#[derive(Debug)]
pub struct Sender<Head, Current, Tail>(Ptr<Head, Current, Tail>)
where
    Head: Concat<(Current,)>,
    <Head as Concat<(Current,)>>::Output: Concat<Tail>,
    Tail: Tuple,
    Whole<Head, Current, Tail>: Tuple,
    <Whole<Head, Current, Tail> as Tuple>::TupleList: InElement;

// SAFETY: We satisfy the contract by exposing no reference to any associated
// function, and provide an atomic algorithm during its access or dropping
// process, which satisfies the need of `Send`.
unsafe impl<Head, Current, Tail> Send for Sender<Head, Current, Tail>
where
    Head: Concat<(Current,)> + Send,
    Current: Send,
    <Head as Concat<(Current,)>>::Output: Concat<Tail>,
    Tail: Tuple + Send,
    Whole<Head, Current, Tail>: Tuple,
    <Whole<Head, Current, Tail> as Tuple>::TupleList: InElement,
{
}

type CurIndex<Head> = <<Head as Tuple>::TupleList as Count>::Count;

impl<Head, Current, Tail> Sender<Head, Current, Tail>
where
    Head: Concat<(Current,)>,
    <Head as Concat<(Current,)>>::Output: Concat<Tail>,
    Tail: Tuple,
    Whole<Head, Current, Tail>: Tuple,
    <Whole<Head, Current, Tail> as Tuple>::TupleList: InElement,
{
    /// # Safety
    ///
    /// `inner` must hold a valid immutable reference to `Inner`.
    unsafe fn new(inner: Ptr<Head, Current, Tail>) -> Self {
        Sender(inner)
    }

    /// Place the value into the slot, or obtain the resulting iterator if no
    /// other senders exist any longer.
    pub fn send(self, value: Current) -> Result<(), Take<Head, Current, Tail>>
    where
        <Head as Tuple>::TupleList: Count,
        Place<Head, Current, Tail>: Index<CurIndex<Head>, Output = Element<Current>>,
    {
        let pointer = self.0;
        // SAFETY: See contract 1 in `Self::new`.
        let inner = unsafe { pointer.as_ref() };
        let elem: &Element<Current> = Index::<CurIndex<Head>>::index(&inner.place);

        // SAFETY: Each sender has its ownership of one `Element` storage in its
        // `inner`, and thus the placing is safe. Besides, the appending `Release`
        // ordering is supplied.
        unsafe { elem.place(value) };
        let fetch_sub = inner.count.fetch_sub(1, Release);

        // We don't want to call the dropper anymore because it decreases the reference
        // count once more.
        mem::forget(self);

        if fetch_sub == 1 {
            // SAFETY: We use `Acquire` fence here to observe other executions of placing
            // values. And since the reference count is now 0, we owns `inner`, so it can be
            // handed to the iterator safely.
            atomic::fence(Acquire);
            return Err(unsafe { Inner::drop_in_place(pointer) });
        }
        Ok(())
    }
}

impl<Head, Current, Tail> Drop for Sender<Head, Current, Tail>
where
    Head: Concat<(Current,)>,
    <Head as Concat<(Current,)>>::Output: Concat<Tail>,
    Tail: Tuple,
    Whole<Head, Current, Tail>: Tuple,
    <Whole<Head, Current, Tail> as Tuple>::TupleList: InElement,
{
    fn drop(&mut self) {
        let pointer = self.0;
        // SAFETY: See contract 1 in `Self::new`.
        let inner = unsafe { pointer.as_ref() };
        // No additional ordering is used because we now have no more
        // observations/modifications to slot values, except...
        if inner.count.fetch_sub(1, Relaxed) == 1 {
            // SAFETY: ... we now owns our `inner`.
            atomic::fence(Acquire);
            unsafe { Inner::drop_in_place(pointer) };
        }
    }
}

/// Create a tuple slot, and return a tuple of senders targeting their own
/// respective element in the slot.
///
/// # Examples
///
/// ```rust
/// let (s1, s2, s3) = either_slot::tuple::<(&str, u8, char)>();
/// s1.send("1").unwrap();
/// s2.send(2).unwrap();
/// let ret = s3.send('3').unwrap_err();
/// assert_eq!(ret, (Some("1"), Some(2), Some('3')));
/// ```
///
/// ```rust
/// let (s1, s2, s3) = either_slot::tuple::<(i32, u8, char)>();
/// drop(s1);
/// s3.send('3').unwrap();
/// let ret = s2.send(2).unwrap_err();
/// assert_eq!(ret, (None, Some(2), Some('3')));
/// ```
pub fn tuple<T>() -> <T::Sender as TupleList>::Tuple
where
    T: Construct,
    <T as Tuple>::TupleList: InElement,
{
    let inner = Inner::<T::TupleList>::new();
    unsafe { T::construct(inner) }.into_tuple()
}

#[cfg(test)]
mod tests {
    #[cfg(not(loom))]
    use std::thread;

    #[cfg(loom)]
    use loom::thread;

    use super::tuple;

    #[test]
    fn send() {
        fn inner() {
            let (s1, s2, s3) = tuple::<(i32, u8, char)>();
            let j2 = thread::spawn(|| s2.send(2));
            let j3 = thread::spawn(|| s3.send('3'));

            let res = s1.send(1).and(j2.join().unwrap()).and(j3.join().unwrap());
            assert_eq!(res, Err((Some(1), Some(2), Some('3'))));
        }

        #[cfg(not(loom))]
        inner();
        #[cfg(loom)]
        loom::model(inner);
    }

    #[test]
    fn drop_one() {
        fn inner() {
            let (s1, s2, s3) = tuple::<(i32, u8, char)>();
            let j2 = thread::spawn(|| {
                drop(s2);
                Ok(())
            });
            let j3 = thread::spawn(|| s3.send('3'));

            let res = s1.send(1).and(j2.join().unwrap()).and(j3.join().unwrap());
            if let Err(r) = res {
                assert_eq!(r, (Some(1), None, Some('3')));
            }
        }

        #[cfg(not(loom))]
        inner();
        #[cfg(loom)]
        loom::model(inner);
    }
}
