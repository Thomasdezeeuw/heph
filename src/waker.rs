//! Module containing the futures `Wake` implementation.

use std::mem;
use std::cell::RefCell;
use std::ptr::NonNull;
use std::rc::Rc;
use std::task::{LocalWaker, Waker, UnsafeWake};

use mio_st::event::Ready;
use mio_st::registration::Notifier;

/// Create a new `LocalWaker`.
///
/// The provided `notifier` must at least have `Ready::READABLE` interest.
///
/// # Safety
///
/// This `LocalWaker` implement `Send` + `Sync` but it shouldn't! It absolutely
/// not `Send` or `Sync`.
pub fn new_waker(notifier: Notifier) -> LocalWaker {
    let waker = Rc::new(RcWaker {
        notifier: RefCell::new(notifier),
    });

    let ptr: *const dyn UnsafeWake = Rc::into_raw(waker);
    let ptr = unsafe { NonNull::new_unchecked(ptr as *mut _) };
    unsafe { LocalWaker::new(ptr) }
}

#[derive(Debug)]
#[repr(transparent)]
struct RcWaker {
    /// The thing to notify when wake is called.
    notifier: RefCell<Notifier>,
}

// Expects `&self` to always a raw pointer to `Rc<RcWaker>`.
unsafe impl UnsafeWake for RcWaker {
    unsafe fn clone_raw(&self) -> Waker {
        let this: Rc<RcWaker> = Rc::from_raw(self);
        // Clone self and turn the clone into a raw pointer,
        let ptr: *const dyn UnsafeWake = Rc::into_raw(this.clone());
        let ptr = NonNull::new_unchecked(ptr as *mut _);
        // Forget self to not decreased the count, we only have a reference here
        // after all.
        mem::forget(this);
        // Return the new `Waker`.
        Waker::new(ptr)
    }

    unsafe fn drop_raw(&self) {
        drop(Rc::from_raw(self));
    }

    unsafe fn wake(&self) {
        // Can't deal with the error here, so we have to ignore it.
        let _ = self.notifier.borrow_mut().notify(Ready::READABLE);
    }
}

// FIXME: This is very unsafe.
unsafe impl Sync for RcWaker { }
unsafe impl Send for RcWaker { }
