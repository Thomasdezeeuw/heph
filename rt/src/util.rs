//! Module with various utilities.

use std::async_iter::AsyncIterator;
use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::mem::forget;
use std::num::NonZero;
use std::pin::Pin;
use std::ptr::NonNull;
use std::task::{self, Poll};

/// Helper [`Future`] that poll `future1` and `future2` and returns the output
/// of the future that completes first.
pub const fn either<Fut1, Fut2>(future1: Fut1, future2: Fut2) -> Either<Fut1, Fut2> {
    Either { future1, future2 }
}

/// The [`Future`] behind [`either`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct Either<Fut1, Fut2> {
    future1: Fut1,
    future2: Fut2,
}

impl<Fut1, Fut2> Future for Either<Fut1, Fut2>
where
    Fut1: Future,
    Fut2: Future,
{
    type Output = Result<Fut1::Output, Fut2::Output>;

    fn poll(mut self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        // SAFETY: not moving `future1`.
        let future1 = unsafe { Pin::map_unchecked_mut(self.as_mut(), |s| &mut s.future1) };
        match future1.poll(ctx) {
            Poll::Ready(value) => Poll::Ready(Ok(value)),
            Poll::Pending => {
                // SAFETY: not moving `future2`.
                let future2 = unsafe { Pin::map_unchecked_mut(self, |s| &mut s.future2) };
                match future2.poll(ctx) {
                    Poll::Ready(value) => Poll::Ready(Err(value)),
                    Poll::Pending => Poll::Pending,
                }
            }
        }
    }
}

/// Returns a [`Future`] that get the next item from `iter`.
pub const fn next<I>(iter: I) -> Next<I> {
    Next { iter }
}

/// The [`Future`] behind [`either`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct Next<I> {
    iter: I,
}

impl<I> Future for Next<I>
where
    I: AsyncIterator,
{
    type Output = Option<I::Item>;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        // SAFETY: not moving `iter`.
        unsafe { Pin::map_unchecked_mut(self, |s| &mut s.iter).poll_next(ctx) }
    }
}

/// Tagged pinned pointer (`Pin<Box<T>>`) to either `L` or `R`.
pub(crate) struct TaggedPointer<L, R> {
    /// This is actually either a `Pin<Box<L>>` or `Pin<Box<R>>`.
    tagged_ptr: NonNull<()>,
    _phantom_data: PhantomData<(*const L, *const R)>,
}

impl TaggedPointer<(), ()> {
    /// Number of bits used for the tag.
    pub(crate) const TAG_BITS: usize = 1;
    const MASK: usize = (1 << TaggedPointer::TAG_BITS) - 1;
    const TAG_LEFT: usize = 0b1;
    const TAG_RIGHT: usize = 0b0;
}

impl<L, R> TaggedPointer<L, R> {
    pub(crate) fn left(left: Pin<Box<L>>) -> TaggedPointer<L, R> {
        // SAFETY: keeping `left` pinned within usage of `TaggedPointer`.
        let ptr = Box::into_non_null(unsafe { Pin::into_inner_unchecked(left) }).cast();
        TaggedPointer::new(ptr, TaggedPointer::TAG_LEFT)
    }

    pub(crate) fn right(right: Pin<Box<R>>) -> TaggedPointer<L, R> {
        // SAFETY: keeping `right` pinned within usage of `TaggedPointer`.
        let ptr = Box::into_non_null(unsafe { Pin::into_inner_unchecked(right) }).cast();
        TaggedPointer::new(ptr, TaggedPointer::TAG_RIGHT)
    }

    /// # Safety
    ///
    /// `ptr` must come from `Box::into_non_null`.
    fn new(ptr: NonNull<()>, tag: usize) -> TaggedPointer<L, R> {
        TaggedPointer {
            tagged_ptr: ptr.map_addr(|ptr| ptr | tag),
            _phantom_data: PhantomData,
        }
    }

    /// Attempts to take `L` from `this`, or returns a mutable reference to `R`.
    ///
    /// Returns:
    /// * `None` if `this` is `None`, `this` is unchanged.
    /// * `Some(Ok(..))` if the pointer is `Some` and points to a process,
    ///   `this` will be `None`.
    /// * `Some(Err(..))` if the pointer is `Some` and points to a branch,
    ///   `this` is unchanged.
    pub(crate) fn take_left<'a>(
        this: &'a mut Option<TaggedPointer<L, R>>,
    ) -> Option<Result<Pin<Box<L>>, Pin<&'a mut R>>> {
        match this {
            Some(ptr) if ptr.is_left() => {
                let ptr = ptr.as_ptr();
                // To avoid a double free we need forget about `this` as it's
                // being converted into a `Box` below.
                forget(this.take());
                let l = unsafe { Box::from_raw(ptr.cast()) };
                Some(Ok(Box::into_pin(l)))
            }
            Some(ptr) => {
                debug_assert!(ptr.is_right());
                let r: &mut R = unsafe { &mut *(ptr.as_ptr().cast()) };
                Some(Err(unsafe { Pin::new_unchecked(r) }))
            }
            None => None,
        }
    }

    /// Returns the raw pointer without its tag.
    fn as_ptr(&self) -> *mut () {
        self.tagged_ptr
            .as_ptr()
            .map_addr(|ptr| ptr & !TaggedPointer::MASK)
    }

    /// Returns the raw pointer without its tag.
    fn as_non_null(&self) -> NonNull<()> {
        // SAFETY: We're guaranteed that removing the tag doesn't result in a
        // null pointer, thus `NonZero::new_unchecked` is safe.
        self.tagged_ptr
            .map_addr(|ptr| unsafe { NonZero::new_unchecked(ptr.get() & !TaggedPointer::MASK) })
    }

    /// Returns `true` is the tagged pointer points to `L`.
    fn is_left(&self) -> bool {
        (self.tagged_ptr.as_ptr().addr() & TaggedPointer::MASK) == TaggedPointer::TAG_LEFT
    }

    /// Returns `true` is the tagged pointer points to `R`.
    fn is_right(&self) -> bool {
        (self.tagged_ptr.as_ptr().addr() & TaggedPointer::MASK) == TaggedPointer::TAG_RIGHT
    }
}

impl<L, R> Drop for TaggedPointer<L, R> {
    fn drop(&mut self) {
        let ptr = self.as_non_null();
        if self.is_left() {
            let left: Box<L> = unsafe { Box::from_non_null(ptr.cast()) };
            drop(left);
        } else {
            debug_assert!(self.is_right());
            let right: Box<R> = unsafe { Box::from_non_null(ptr.cast()) };
            drop(right);
        }
    }
}

impl<L: fmt::Debug, R: fmt::Debug> fmt::Debug for TaggedPointer<L, R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ptr = self.as_ptr();
        if self.is_left() {
            let left: &L = unsafe { &*ptr.cast() };
            left.fmt(f)
        } else {
            debug_assert!(self.is_right());
            let right: &R = unsafe { &*ptr.cast() };
            right.fmt(f)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::mem::size_of;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::TaggedPointer;

    #[test]
    fn tagged_pointer_size_assertions() {
        assert_eq!(size_of::<TaggedPointer<(), ()>>(), size_of::<*const ()>());
        // `L` and `R` types shouldn't matter.
        assert_eq!(
            size_of::<TaggedPointer<String, usize>>(),
            size_of::<*const ()>()
        );
    }

    #[test]
    fn tagged_pointer_take_left() {
        // None.
        let mut ptr: Option<TaggedPointer<String, usize>> = None;
        assert!(TaggedPointer::take_left(&mut ptr).is_none());
        assert!(ptr.is_none());

        // L -> Some(Ok(..)).
        let mut ptr: Option<TaggedPointer<String, usize>> =
            Some(TaggedPointer::left(Box::pin(String::from("Hello"))));
        match TaggedPointer::take_left(&mut ptr) {
            Some(Ok(_)) => {}
            _ => panic!("unexpected result"),
        }
        // L is removed.
        assert!(ptr.is_none());

        // R -> Some(Err(..)).
        let mut ptr: Option<TaggedPointer<String, usize>> = Some(TaggedPointer::right(Box::pin(1)));
        match TaggedPointer::take_left(&mut ptr) {
            Some(Err(_)) => {}
            _ => panic!("unexpected result"),
        }
        // Pointer unchanged.
        assert!(ptr.is_some());
    }

    #[test]
    fn tagged_pointer_drop() {
        // This shouldn't panic or anything.
        let ptr = TaggedPointer::<String, usize>::left(Box::pin(String::from("Hello, World!")));
        drop(ptr);

        struct DropTest(Arc<AtomicUsize>);

        impl Drop for DropTest {
            fn drop(&mut self) {
                let _ = self.0.fetch_add(1, Ordering::AcqRel);
            }
        }

        let dropped_left = Arc::new(AtomicUsize::new(0));
        let dropped_right = Arc::new(AtomicUsize::new(0));

        let ptr_left = TaggedPointer::<_, usize>::left(Box::pin(DropTest(dropped_left.clone())));
        let ptr_right = TaggedPointer::<usize, _>::right(Box::pin(DropTest(dropped_right.clone())));

        assert_eq!(dropped_left.load(Ordering::Acquire), 0);
        drop(ptr_left);
        assert_eq!(dropped_left.load(Ordering::Acquire), 1);

        assert_eq!(dropped_right.load(Ordering::Acquire), 0);
        drop(ptr_right);
        assert_eq!(dropped_right.load(Ordering::Acquire), 1);
    }
}
