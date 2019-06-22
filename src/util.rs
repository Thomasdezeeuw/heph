//! Module with utilities used throughout the crate.

use std::cell::{Ref, RefCell, RefMut};
use std::rc::{Rc, Weak};

/// A `Rc<RefCell<T>>` with some easy to use methods.
#[derive(Debug)]
#[repr(transparent)]
pub struct Shared<T> {
    inner: Rc<RefCell<T>>,
}

impl<T> Shared<T> {
    /// Create a new shared value.
    pub fn new(value: T) -> Shared<T> {
        Shared {
            inner: Rc::new(RefCell::new(value)),
        }
    }

    /// Borrow the value, i.e. `&T`.
    pub fn borrow(&self) -> Ref<'_, T> {
        match self.inner.try_borrow() {
            Ok(inner) => inner,
            Err(_) => unreachable!("tried to borrow an already borrowed Shared"),
        }
    }

    /// Mutably borrow the value, i.e. `&mut T`.
    pub fn borrow_mut(&mut self) -> RefMut<'_, T> {
        match self.inner.try_borrow_mut() {
            Ok(inner) => inner,
            Err(_) => unreachable!("tried to mutable borrow an already borrowed Shared"),
        }
    }
}

impl<T> Clone for Shared<T> {
    fn clone(&self) -> Shared<T> {
        Shared {
            inner: Rc::clone(&self.inner),
        }
    }
}

/// A `Weak<RefCell<T>>` with some easy to use methods.
#[derive(Debug)]
#[repr(transparent)]
pub struct WeakShared<T> {
    /// Note: if this representation changes it will break the Actor Registry!
    inner: Weak<RefCell<T>>,
}

impl<T> Clone for WeakShared<T> {
    fn clone(&self) -> WeakShared<T> {
        WeakShared {
            inner: Weak::clone(&self.inner),
        }
    }
}
