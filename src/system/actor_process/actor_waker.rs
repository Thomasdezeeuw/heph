//! Module containing the futures' `Waker` implementation for an `ActorProcess`.

use std::sync::Arc;
use std::cell::RefCell;

use futures_core::task::{Wake, Waker};
use mio_st::registration::Notifier;
use mio_st::event::Ready;

#[derive(Debug)]
pub struct ActorProcessWaker {
    wake: Arc<ActorProcessWake>,
}

impl ActorProcessWaker {
    /// Create a new `ActorProcessWaker`.
    pub fn new(notifier: Notifier) -> ActorProcessWaker  {
        ActorProcessWaker {
            wake: Arc::new(ActorProcessWake {
                notifier: RefCell::new(notifier),
            }),
        }
    }

    /// Create a new `Waker`.
    pub fn waker(&mut self) -> Waker {
        Waker::from(Arc::clone(&self.wake))
    }
}

/// Our `Wake` implementation.
#[derive(Debug)]
struct ActorProcessWake {
    notifier: RefCell<Notifier>,
}

impl Wake for ActorProcessWake {
    fn wake(arc_self: &Arc<Self>) {
        // Can't deal with error, so we have to ignore the error.
        let _ = arc_self.notifier.borrow_mut().notify(Ready::READABLE);
    }
}

// This is incorrect.
unsafe impl Send for ActorProcessWake {}
unsafe impl Sync for ActorProcessWake {}
