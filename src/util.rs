//! Module with utilities used throughout the crate.

use std::cell::{RefCell, Ref, RefMut};
use std::rc::Rc;

/// A `Rc<RefCell<T>>` with some easy to use methods.
#[derive(Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(transparent)]
pub(crate) struct Shared<T> {
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
    pub fn borrow(&self) -> Ref<T> {
        match self.inner.try_borrow() {
            Ok(inner) => inner,
            Err(_) => unreachable!("tried to borrow an already borrowed Shared"),
        }
    }

    /// Mutably borrow the value, i.e. `&mut T`.
    pub fn borrow_mut(&mut self) -> RefMut<T> {
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
