//! Module with shared runtime internals.

use std::cell::RefCell;
use std::num::NonZeroUsize;
use std::sync::Arc;

use heph::actor_ref::ActorGroup;
use mio::Poll;

use crate::{shared, trace, Signal};

mod scheduler;
pub(super) mod waker;

use crate::timers::Timers;
use scheduler::Scheduler;
use waker::WakerId;

/// Internals of the runtime, to which `RuntimeRef`s have a reference.
#[derive(Debug)]
pub(super) struct RuntimeInternals {
    /// Unique id among the worker threads.
    pub(super) id: NonZeroUsize,
    /// Runtime internals shared between coordinator and worker threads.
    pub(super) shared: Arc<shared::RuntimeInternals>,
    /// Waker id used to create a `Waker` for thread-local actors.
    pub(super) waker_id: WakerId,
    /// Scheduler for thread-local actors.
    pub(super) scheduler: RefCell<Scheduler>,
    /// OS poll, used for event notifications to support non-blocking I/O.
    pub(super) poll: RefCell<Poll>,
    /// I/O uring.
    pub(super) ring: RefCell<a10::Ring>,
    /// Timers, deadlines and timeouts.
    pub(crate) timers: RefCell<Timers>,
    /// Actor references to relay received `Signal`s to.
    pub(super) signal_receivers: RefCell<ActorGroup<Signal>>,
    /// CPU affinity of the worker thread, or `None` if not set.
    pub(super) cpu: Option<usize>,
    /// Log used for tracing, `None` is tracing is disabled.
    pub(super) trace_log: RefCell<Option<trace::Log>>,
}

impl RuntimeInternals {
    /// Create a local runtime internals.
    pub(super) fn new(
        id: NonZeroUsize,
        shared_internals: Arc<shared::RuntimeInternals>,
        waker_id: WakerId,
        poll: Poll,
        ring: a10::Ring,
        cpu: Option<usize>,
        trace_log: Option<trace::Log>,
    ) -> RuntimeInternals {
        RuntimeInternals {
            id,
            shared: shared_internals,
            waker_id,
            scheduler: RefCell::new(Scheduler::new()),
            poll: RefCell::new(poll),
            ring: RefCell::new(ring),
            timers: RefCell::new(Timers::new()),
            signal_receivers: RefCell::new(ActorGroup::empty()),
            cpu,
            trace_log: RefCell::new(trace_log),
        }
    }
}
