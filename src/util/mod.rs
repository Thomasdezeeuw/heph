//! Module via utility types.

use std::marker::PhantomData;
use std::mem::forget;
use std::pin::Pin;
use std::ptr::NonNull;

// TODO: add alignment test to show the tag bit isn't used.

/// A pointer that points to one of two types, `L` or `R`.
pub struct TaggedBox<L, R> {
    /// Pointer to the allocation, tagged using `LEFT_TAG` or `RIGHT_TAG`.
    tagged_ptr: NonNull<()>,
    _phantom: PhantomData<(L, R)>,
}

/// Tag used for the pointer.
const LEFT_TAG: usize = 0b1;
const RIGHT_TAG: usize = 0b0;

/// Its either left or right, what will it be?
pub enum Either<L, R> {
    Left(L),
    Right(R),
}

impl<L, R> Either<L, R> {
    /// Unwrap the left side, panicking if its the right side.
    pub fn unwrap_left(self) -> L {
        match self {
            Either::Left(left) => left,
            Either::Right(_) => panic!("tried to unwrap the left side, but got a right side"),
        }
    }

    /// Unwrap the right side, panicking if its the left side.
    pub fn unwrap_right(self) -> R {
        match self {
            Either::Left(_) => panic!("tried to unwrap the right side, but got a left side"),
            Either::Right(right) => right,
        }
    }

    /// Returns `true` if self is the left variant.
    #[cfg(test)]
    fn is_left(&self) -> bool {
        match self {
            Either::Left(_) => true,
            Either::Right(_) => false,
        }
    }
}

impl<L, R> TaggedBox<L, R> {
    /// Convert `left` into a `TaggedBox`.
    pub fn new_left(left: Pin<Box<L>>) -> TaggedBox<L, R> {
        let ptr = unsafe { Box::into_raw(Pin::into_inner_unchecked(left)) };
        TaggedBox::new(ptr as *mut _, LEFT_TAG)
    }

    /// Convert `right` into a `TaggedBox`.
    pub fn new_right(right: Pin<Box<R>>) -> TaggedBox<L, R> {
        let ptr = unsafe { Box::into_raw(Pin::into_inner_unchecked(right)) };
        TaggedBox::new(ptr as *mut _, RIGHT_TAG)
    }

    /// Create a new `TaggedBox` for `ptr` using `tag`.
    const fn new(ptr: *mut (), tag: usize) -> TaggedBox<L, R> {
        #[allow(trivial_casts)]
        let tagged_ptr = unsafe { ((ptr as *mut _ as usize) | tag) as *mut () };
        TaggedBox {
            tagged_ptr: unsafe { NonNull::new_unchecked(tagged_ptr) },
            _phantom: PhantomData,
        }
    }

    /*
    /// Dereference the pointer.
    pub fn deref(&self) -> Either<Pin<&L>, Pin<&R>> {
        let ptr = self.as_ptr();
        if self.is_left() {
            let left: &L = unsafe { &*(ptr as *const _) };
            Either::Left(unsafe { Pin::new_unchecked(left) })
        } else {
            let right: &R = unsafe { &*(ptr as *const _) };
            Either::Right(unsafe { Pin::new_unchecked(right) })
        }
    }
    */

    /// Mutably dereference the pointer.
    pub fn deref_mut(&mut self) -> Either<Pin<&mut L>, Pin<&mut R>> {
        let ptr = self.as_ptr();
        if self.is_left() {
            let left: &mut L = unsafe { &mut *(ptr as *mut _) };
            Either::Left(unsafe { Pin::new_unchecked(left) })
        } else {
            let right: &mut R = unsafe { &mut *(ptr as *mut _) };
            Either::Right(unsafe { Pin::new_unchecked(right) })
        }
    }

    /// Return the original pointer this `TaggedBox` was created with.
    pub fn into_inner(self) -> Either<Pin<Box<L>>, Pin<Box<R>>> {
        let ptr = self.as_ptr();
        let ret = if self.is_left() {
            let left: Pin<Box<L>> = unsafe { Pin::new_unchecked(Box::from_raw(ptr as *mut _)) };
            Either::Left(left)
        } else {
            let right: Pin<Box<R>> = unsafe { Pin::new_unchecked(Box::from_raw(ptr as *mut _)) };
            Either::Right(right)
        };
        // Don't drop the allocation.
        forget(self);
        ret
    }

    /// Returns `true` if the pointer points to `L`.
    fn is_left(&self) -> bool {
        (self.tagged_ptr.as_ptr() as usize & LEFT_TAG) != 0
    }

    /// Returns the pointer.
    fn as_ptr(&self) -> *mut () {
        (self.tagged_ptr.as_ptr() as usize & !(LEFT_TAG)) as *mut ()
    }
}

impl<L, R> Drop for TaggedBox<L, R> {
    fn drop(&mut self) {
        let ptr = self.as_ptr();
        if self.is_left() {
            let left: Pin<Box<L>> = unsafe { Pin::new_unchecked(Box::from_raw(ptr as *mut _)) };
            drop(left);
        } else {
            let right: Pin<Box<R>> = unsafe { Pin::new_unchecked(Box::from_raw(ptr as *mut _)) };
            drop(right);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::TaggedBox;

    type T = TaggedBox<usize, bool>;

    #[test]
    fn smoke() {
        let mut left = Box::pin(123);
        let left_ptr: *mut usize = &mut *left;
        let mut right = Box::pin(false);
        let right_ptr: *mut bool = &mut *right;

        let mut left: T = TaggedBox::new_left(left);
        let mut right: T = TaggedBox::new_right(right);

        assert!(left.is_left());
        assert!(!right.is_left());

        assert_eq!(left.as_ptr() as *mut _, left_ptr);
        assert_eq!(right.as_ptr() as *mut _, right_ptr);

        assert!(left.deref_mut().is_left());
        assert!(!right.deref_mut().is_left());

        assert!(left.deref_mut().is_left());
        assert!(!right.deref_mut().is_left());

        let mut left = left.into_inner().unwrap_left();
        let mut right = right.into_inner().unwrap_right();

        #[allow(trivial_casts)]
        {
            assert_eq!(&mut *left as *mut _, left_ptr);
            assert_eq!(&mut *right as *mut _, right_ptr);
        }
    }

    #[test]
    fn drop_test() {
        // This shouldn't panic or anything.
        let ptr: T = TaggedBox::new_left(Box::pin(123));
        drop(ptr);

        struct DropTest(Arc<AtomicUsize>);

        impl Drop for DropTest {
            fn drop(&mut self) {
                let _ = self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let dropped_left = Arc::new(AtomicUsize::new(0));
        let dropped_right = Arc::new(AtomicUsize::new(0));

        let left: TaggedBox<DropTest, ()> =
            TaggedBox::new_left(Box::pin(DropTest(dropped_left.clone())));
        let right: TaggedBox<(), DropTest> =
            TaggedBox::new_right(Box::pin(DropTest(dropped_right.clone())));

        assert_eq!(dropped_left.load(Ordering::Acquire), 0);
        assert_eq!(dropped_right.load(Ordering::Acquire), 0);

        drop(left);
        drop(right);

        assert_eq!(dropped_left.load(Ordering::Acquire), 1);
        assert_eq!(dropped_right.load(Ordering::Acquire), 1);
    }
}
