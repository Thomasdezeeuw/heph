//! Module containing the process related types and implementations.

use std::cmp::Ordering;
use std::fmt;
use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::task::{self, Poll};
use std::time::{Duration, Instant};

use heph::supervisor::Supervisor;
use heph::{ActorFuture, NewActor};
use log::{error, trace};

use crate::panic_message;
use crate::spawn::options::Priority;

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
        // SAFETY: not moving the `Fut`ure.
        let future = unsafe { Pin::map_unchecked_mut(self.as_mut(), |s| &mut s.0) };
        match catch_unwind(AssertUnwindSafe(|| future.poll(ctx))) {
            Ok(Poll::Ready(())) => Poll::Ready(()),
            Ok(Poll::Pending) => Poll::Pending,
            Err(panic) => {
                let msg = panic_message(&*panic);
                let name = self.name();
                error!("future '{name}' panicked at '{msg}'");
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

/// Process container. Hold the process itself and all data needed to properly
/// schedule and run it.
///
/// # Notes
///
/// `PartialEq` and `Eq` are implemented based on the id of the process
/// (`ProcessId`).
///
/// `PartialOrd` and `Ord` however are implemented based on runtime and
/// priority.
pub(crate) struct Process<P: ?Sized> {
    priority: Priority,
    /// Fair runtime of the process, which is `actual runtime * priority`.
    fair_runtime: Duration,
    process: Pin<Box<P>>,
}

impl<P: ?Sized> Process<P> {
    /// Create a new process container.
    pub(crate) const fn new(priority: Priority, process: Pin<Box<P>>) -> Process<P> {
        Process {
            priority,
            fair_runtime: Duration::ZERO,
            process,
        }
    }

    /// Returns the process identifier, or pid for short.
    pub(crate) fn id(&self) -> ProcessId {
        ProcessId((&raw const *self).addr())
    }

    #[cfg(test)]
    pub(crate) fn set_fair_runtime(&mut self, fair_runtime: Duration) {
        self.fair_runtime = fair_runtime;
    }

    #[cfg(test)]
    pub(crate) fn fair_runtime(&mut self) -> Duration {
        self.fair_runtime
    }
}

impl<P: Run + ?Sized> Process<P> {
    /// Returns the name of the process.
    pub(crate) fn name(&self) -> &'static str {
        self.process.name()
    }

    /// Run the process.
    ///
    /// Returns the completion state of the process.
    pub(crate) fn run(&mut self, ctx: &mut task::Context<'_>) -> RunStats {
        let pid = self.id();
        let name = self.process.name();
        trace!(pid = pid.0, name = name; "running process");

        let start = Instant::now();
        let result = self.process.as_mut().run(ctx);
        let elapsed = start.elapsed();
        let fair_elapsed = elapsed * self.priority;
        self.fair_runtime += fair_elapsed;

        trace!(
            pid = pid.0, name = name, elapsed:? = elapsed, result:? = result;
            "finished running process",
        );
        RunStats { elapsed, result }
    }
}

/// Statistics about the process run.
#[derive(Copy, Clone, Debug)]
#[must_use = "Must check the process's `result`"]
pub(crate) struct RunStats {
    /// The duration for which the process ran.
    pub(crate) elapsed: Duration,
    /// The result of the process run.
    pub(crate) result: Poll<()>,
}

impl<P: ?Sized> Eq for Process<P> {}

impl<P: ?Sized> PartialEq for Process<P> {
    fn eq(&self, other: &Self) -> bool {
        Pin::new(self).id() == Pin::new(other).id()
    }
}

impl<P: ?Sized> Ord for Process<P> {
    fn cmp(&self, other: &Self) -> Ordering {
        (other.fair_runtime)
            .cmp(&(self.fair_runtime))
            .then_with(|| self.priority.cmp(&other.priority))
    }
}

impl<P: ?Sized> PartialOrd for Process<P> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[allow(clippy::missing_fields_in_debug)]
impl<P: Run + ?Sized> fmt::Debug for Process<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Process")
            .field("id", &self.id())
            .field("name", &self.name())
            .field("priority", &self.priority)
            .field("fair_runtime", &self.fair_runtime)
            .finish()
    }
}
