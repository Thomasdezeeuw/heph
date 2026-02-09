//! Module containing the process related types and implementations.

use std::cmp::Ordering;
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::task::{self, Poll};
use std::time::{Duration, Instant};
use std::{fmt, ptr};

use heph::supervisor::Supervisor;
use heph::{ActorFuture, NewActor};
use log::trace;

use crate::panic_message;
use crate::scheduler::{Priority, Schedule};

/// Process id, or pid for short, is an identifier for a process in the runtime.
///
/// This can only be created by one of the schedulers and should be seen as an
/// opaque type for the rest of the crate.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Ord, PartialOrd)]
#[repr(transparent)]
pub(crate) struct ProcessId(pub(crate) usize);

impl fmt::Debug for ProcessId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Display for ProcessId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl log::kv::ToValue for ProcessId {
    fn to_value(&self) -> log::kv::Value<'_> {
        self.0.to_value()
    }
}

/// A runnable process.
pub(crate) trait Run {
    /// Return the name of this process, used in logging.
    fn name(&self) -> &'static str;

    /// Run the process until it can't make any more progress without blocking.
    ///
    /// See [`Future::poll`].
    ///
    /// # Panics
    ///
    /// The implementation MUST catch panics.
    fn run(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<()>;
}

/// Wrapper around a [`Future`] to implement [`Process`].
pub(crate) struct FutureProcess<Fut>(pub(crate) Fut);

impl<Fut: Future<Output = ()>> Run for FutureProcess<Fut> {
    fn name(&self) -> &'static str {
        "FutureProcess"
    }

    fn run(mut self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<()> {
        // SAFETY: not moving the future.
        let future = unsafe { Pin::map_unchecked_mut(self.as_mut(), |s| &mut s.0) };
        match catch_unwind(AssertUnwindSafe(|| future.poll(ctx))) {
            Ok(Poll::Ready(())) => Poll::Ready(()),
            Ok(Poll::Pending) => Poll::Pending,
            Err(panic) => {
                let msg = panic_message(&*panic);
                let name = self.name();
                log::error!("future '{name}' panicked at '{msg}'");
                Poll::Ready(())
            }
        }
    }
}

impl<S, NA> Run for ActorFuture<S, NA>
where
    S: Supervisor<NA>,
    NA: NewActor,
    NA::RuntimeAccess: Clone,
{
    fn name(&self) -> &'static str {
        NA::name()
    }

    fn run(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<()> {
        // NOTE: `ActorFuture` already catches panics for us.
        self.poll(ctx)
    }
}

/// Process container.
///
/// Holds the process itself and all data needed to properly schedule and run
/// it.
///
/// # Notes
///
/// `PartialEq` and `Eq` are implemented based on the id of the process
/// (`ProcessId`).
///
/// `PartialOrd` and `Ord` however are implemented using `S::order`.
pub(crate) struct Process<S, P: ?Sized> {
    id: ProcessId,
    scheduler_data: S,
    process: Pin<Box<P>>,
}

impl<S: Schedule, P: Run + ?Sized> Process<S, P> {
    /// Create a new process container.
    pub(crate) fn new(id: ProcessId, priority: Priority, process: Pin<Box<P>>) -> Process<S, P> {
        Process {
            id,
            scheduler_data: S::new(priority),
            process,
        }
    }

    // TODO: remove
    pub(crate) fn set_id(self: &mut Pin<Box<Self>>) {
        let pid = ProcessId(ptr::from_ref(&**self).addr());
        unsafe { self.as_mut().get_unchecked_mut().id = pid };
    }

    /// Run the process.
    ///
    /// Returns the completion state of the process.
    pub(crate) fn run(&mut self, ctx: &mut task::Context<'_>) -> RunStats {
        let pid = self.id();
        let name = self.process.name();
        trace!(pid, name; "running process");

        let start = Instant::now();
        let result = self.process.as_mut().run(ctx);
        let end = Instant::now();
        let elapsed = end - start;
        self.scheduler_data.update(start, end, elapsed);

        trace!(pid, name, elapsed:?, result:?; "finished running process");
        RunStats { elapsed, result }
    }
}

/// Statistics about the run of a process.
#[derive(Copy, Clone, Debug)]
#[must_use = "Must check the process's result"]
pub(crate) struct RunStats {
    /// The duration for which the process ran.
    pub(crate) elapsed: Duration,
    /// The result of the process run.
    pub(crate) result: Poll<()>,
}

impl<S, P: ?Sized> Process<S, P> {
    /// Returns the process identifier, or pid for short.
    pub(crate) fn id(&self) -> ProcessId {
        self.id
    }

    #[cfg(test)]
    pub(crate) fn scheduler_data(&mut self) -> &mut S {
        &mut self.scheduler_data
    }
}

impl<S, P: Run + ?Sized> Process<S, P> {
    /// Returns the name of the process.
    pub(crate) fn name(&self) -> &'static str {
        self.process.name()
    }
}

impl<S, P: ?Sized> Eq for Process<S, P> {}

impl<S, P: ?Sized> PartialEq for Process<S, P> {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
    }
}

impl<S: Schedule, P: ?Sized> Ord for Process<S, P> {
    fn cmp(&self, other: &Self) -> Ordering {
        S::order(&self.scheduler_data, &other.scheduler_data)
    }
}

impl<S: Schedule, P: ?Sized> PartialOrd for Process<S, P> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[allow(clippy::missing_fields_in_debug)]
impl<S: fmt::Debug, P: Run + ?Sized> fmt::Debug for Process<S, P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Process")
            .field("id", &self.id())
            .field("scheduler_data", &self.scheduler_data)
            .field("name", &self.name())
            .finish()
    }
}
