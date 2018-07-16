//! Module containing the futures `Wake` implementation.

use std::cell::RefCell;
use std::sync::Arc;
use std::task::{self, Wake, LocalWaker};

use mio_st::event::Ready;
use mio_st::registration::Notifier;

#[derive(Debug)]
pub struct Waker {
    /// The thing to notify when wake is called.
    notifier: RefCell<Notifier>,
}

impl Waker {
    /// Create a new `LocalWaker` backed by `Waker`.
    ///
    /// The provided `notifier` must at least have `Ready::READABLE` interest.
    pub fn new(notifier: Notifier) -> LocalWaker {
        let waker = Waker { notifier: RefCell::new(notifier) };
        unsafe { task::local_waker(Arc::new(waker)) }
    }
}

impl Wake for Waker {
    fn wake(arc_self: &Arc<Self>) {
        // Can't deal with the error here, so we have to ignore it.
        let _ = arc_self.notifier.borrow_mut().notify(Ready::READABLE);
    }
}

// FIXME: This is very unsafe.
unsafe impl Sync for Waker { }
unsafe impl Send for Waker { }
