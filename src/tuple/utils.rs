use core::{marker::PhantomData, ptr::NonNull};

use tuple_list::{Tuple, TupleList};

use super::{Element, Inner, Sender};

/// A trait for tuple lists that can be converted into its element storage
/// place in [`Sender`].
pub trait InElement: TupleList {
    #[doc(hidden)]
    type Place: TupleList;
    #[doc(hidden)]
    fn init() -> Self::Place;
    /// See [`Element::place`] for more information.
    #[doc(hidden)]
    unsafe fn place(place: &Self::Place, data: Self);

    #[doc(hidden)]
    type Take: TupleList;
    /// See [`Element::take`] for more information.
    #[doc(hidden)]
    unsafe fn take(place: &Self::Place) -> Self::Take;
}

impl InElement for () {
    type Place = ();
    fn init() {}
    unsafe fn place(_: &(), _: ()) {}
    type Take = ();
    unsafe fn take(_: &()) {}
}

impl<Head, Tail> InElement for (Head, Tail)
where
    (Head, Tail): TupleList,
    Tail: InElement,
    (Element<Head>, <Tail as InElement>::Place): TupleList,
    (Option<Head>, <Tail as InElement>::Take): TupleList,
{
    type Place = (Element<Head>, <Tail as InElement>::Place);

    fn init() -> Self::Place {
        (Element::default(), <Tail as InElement>::init())
    }

    unsafe fn place(place: &Self::Place, data: Self) {
        place.0.place(data.0);
        <Tail as InElement>::place(&place.1, data.1);
    }

    type Take = (Option<Head>, <Tail as InElement>::Take);

    unsafe fn take(place: &Self::Place) -> Self::Take {
        let head = place.0.take();
        (head, <Tail as InElement>::take(&place.1))
    }
}

/// A tuple that is concatenable of other tuples.
pub trait Concat<T: Tuple>: Tuple {
    /// The concatenated tuple result.
    type Output: Tuple;

    /// Concatenates 2 tuples into a longer tuple.
    fn concat(self, other: T) -> Self::Output;
}

macro_rules! impl_concat {
    ($($t:ident,)*) => (impl_concat!(@TRANS ($($t,)*), ()););
    (@TRANS ($head:ident, $($a:ident,)*), ()) => {
        impl_concat!(@IMPL (), ($head, $($a,)*));
        impl_concat!(@TRANS ($($a,)*), ());
    };
    (@TRANS (), ()) => {
        impl_concat!(@IMPL (), ());
    };
    (@IMPL ($($b:ident,)*), ($head:ident, $($a:ident,)*)) => {
        impl<$head, $($a,)* $($b,)*> Concat<($head, $($a,)*)> for ($($b,)*)
        where
            ($head, $($a,)*): Tuple,
            ($($b,)*): Tuple,
            ($head, $($a,)* $($b,)*): Tuple,
        {
            type Output = ($($b,)* $head, $($a,)* );

            #[allow(non_snake_case)]
            #[allow(unused_parens)]
            fn concat(self, other: ($head, $($a,)*)) -> Self::Output {
                let ($head, $($a),*) = other;
                let ($($b,)*) = self;
                ($($b,)* $head, $($a,)*)
            }
        }
        impl_concat!(@IMPL ($($b,)* $head,), ($($a,)*));
    };
    (@IMPL ($($b:ident,)*), ()) => {
        impl<$($b,)*> Concat<()> for ($($b,)*)
        where
            ($($b,)*): Tuple,
        {
            type Output = ($($b,)*);

            fn concat(self, _: ()) -> Self::Output {
                self
            }
        }
    }
}

impl_concat!(A, B, C, D, E, F, G, H, I, J, K, L,);

pub struct UTerm;
pub struct UInt<U>(PhantomData<U>);

/// A typenum-countable tuple list.
pub trait Count: TupleList {
    /// The output typenum count.
    type Count;
}

impl Count for () {
    type Count = UTerm;
}

impl<Head, Tail> Count for (Head, Tail)
where
    Tail: Count,
    (Head, Tail): TupleList,
{
    type Count = UInt<<Tail as Count>::Count>;
}

/// A typenum-indexable tuple list.
///
/// The type parameter can only receive the output type of [`Count`].
pub trait Index<I>: TupleList {
    /// The indexed result type.
    type Output;

    /// Index the tuple list and obtains its resulting reference.
    fn index(&self) -> &Self::Output;
}

impl<Head, Tail> Index<UTerm> for (Head, Tail)
where
    Tail: TupleList,
    (Head, Tail): TupleList,
{
    type Output = Head;

    fn index(&self) -> &Self::Output {
        &self.0
    }
}

impl<Head, Tail, U> Index<UInt<U>> for (Head, Tail)
where
    Tail: Index<U>,
    (Head, Tail): TupleList,
{
    type Output = <Tail as Index<U>>::Output;

    fn index(&self) -> &Self::Output {
        self.1.index()
    }
}

/// A tuple type that is constructible into its tuple slot type.
pub trait Construct: Tuple
where
    Self::TupleList: InElement,
{
    /// The constructed tuple list of senders. The returned result type of
    /// [`tuple`](crate::tuple::tuple) is the corresponding tuple type.
    type Sender: TupleList;

    #[allow(private_interfaces)]
    #[doc(hidden)]
    unsafe fn construct(inner: NonNull<Inner<Self::TupleList>>) -> Self::Sender;
}

macro_rules! impl_construct {
    ($($t:ident,)*) => { impl_construct!(@TRANS $($t,)*); };
    (@TRANS $head:ident, $($rest:ident,)*) => {
        impl_construct!(@IMPL ($head, $($rest,)*), ($head, $($rest,)*));
        impl_construct!(@TRANS $($rest,)*);
    };
    (@TRANS) => { impl_construct!(@IMPL (), ()); };
    (@IMPL ($($whole:ident,)*), ($head:ident, $($rest:ident,)*)) => {
        impl<$($whole,)*> Construct for ($($whole,)*) {
            type Sender = impl_construct!(@DEF (), ($head, $($rest,)*));

            #[allow(private_interfaces)]
            unsafe fn construct(inner: NonNull<Inner<Self::TupleList>>) -> Self::Sender {
                impl_construct!(@INIT inner ($head, $($rest,)*))
            }
        }
    };
    (@IMPL (), ()) => {
        impl Construct for () {
            type Sender = ();

            #[allow(private_interfaces)]
            unsafe fn construct(_: NonNull<Inner<Self::TupleList>>) {}
        }
    };
    (@DEF ($($prefix:ident,)*), ($current:ident, $($suffix:ident,)*)) => {
        (
            Sender<($($prefix,)*), $current, ($($suffix,)*)>,
            impl_construct!(@DEF ($($prefix,)* $current,), ($($suffix,)*))
        )
    };
    (@DEF ($($prefix:ident,)*), ()) => (());
    (@INIT $inner:ident ($current:ident, $($suffix:ident,)*)) => {
        (
            Sender::new($inner),
            impl_construct!(@INIT $inner ($($suffix,)*))
        )
    };
    (@INIT $inner:ident ()) => (());
}
impl_construct!(A, B, C, D, E, F, G, H, I, J, K, L,);
