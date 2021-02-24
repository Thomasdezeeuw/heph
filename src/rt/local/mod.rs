//! Module with shared runtime internals.

use std::cell::RefCell;
use std::sync::Arc;

use mio::Poll;

use crate::actor_ref::ActorRef;
use crate::rt::{shared, Signal, WakerId};

mod scheduler;
mod timers;

pub(crate) use scheduler::Scheduler;
pub(crate) use timers::Timers;

/// Internals of the runtime, to which `RuntimeRef`s have a reference.
#[derive(Debug)]
pub(super) struct RuntimeInternals {
    /// Runtime internals shared between coordinator and worker threads.
    pub(super) shared: Arc<shared::RuntimeInternals>,
    /// Waker id used to create a `Waker` for thread-local actors.
    pub(super) waker_id: WakerId,
    /// Scheduler for thread-local actors.
    pub(super) scheduler: RefCell<Scheduler>,
    /// OS poll, used for event notifications to support non-blocking I/O.
    pub(super) poll: RefCell<Poll>,
    /// Timers, deadlines and timeouts.
    pub(crate) timers: RefCell<Timers>,
    /// Actor references to relay received `Signal`s to.
    pub(super) signal_receivers: RefCell<Vec<ActorRef<Signal>>>,
    /// CPU affinity of the worker thread, or `None` if not set.
    pub(super) cpu: Option<usize>,
}

impl RuntimeInternals {
    /// Create a local runtime internals.
    pub(super) fn new(
        shared_internals: Arc<shared::RuntimeInternals>,
        waker_id: WakerId,
        poll: Poll,
        cpu: Option<usize>,
    ) -> RuntimeInternals {
        RuntimeInternals {
            shared: shared_internals,
            waker_id,
            scheduler: RefCell::new(Scheduler::new()),
            poll: RefCell::new(poll),
            timers: RefCell::new(Timers::new()),
            signal_receivers: RefCell::new(Vec::new()),
            cpu,
        }
    }
}
