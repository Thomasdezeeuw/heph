//! Module containing the `Process` trait, related types and implementations.

use std::cmp::Ordering;
use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::task::{self, Poll};
use std::time::{Duration, Instant};
use std::{fmt, ptr};

use heph::supervisor::Supervisor;
use heph::{ActorFuture, NewActor};
use log::{error, trace};

use crate::panic_message;
use crate::spawn::options::Priority;

#[cfg(test)]
mod tests;

/// Process id, or pid for short, is an identifier for a process in the runtime.
///
/// This can only be created by one of the schedulers and should be seen as an
/// opaque type for the rest of the crate.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Ord, PartialOrd)]
#[repr(transparent)]
pub(crate) struct ProcessId(pub(crate) usize);

impl fmt::Debug for ProcessId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for ProcessId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// The trait that represents a process.
///
/// # Panics
///
/// The implementation of the [`Future`] MUST catch panics.
pub(crate) trait Process: Future<Output = ()> {
    /// Return the id for the process.
    fn id(self: Pin<&Self>, alternative: ProcessId) -> ProcessId {
        if size_of_val(&*self) == 0 {
            // For zero sized types Box doesn't make an actual allocation, which
            // means that the pointer will be the same for all zero sized types,
            // i.e. not unique.
            alternative
        } else {
            ProcessId(ptr::addr_of!(*self).cast::<()>() as usize)
        }
    }

    /// Return the name of this process, used in logging.
    fn name(&self) -> &'static str;
}

/// Wrapper around a [`Future`] to implement [`Process`].
pub(crate) struct FutureProcess<Fut>(pub(crate) Fut);

impl<Fut: Future<Output = ()>> Future for FutureProcess<Fut> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
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

impl<Fut> Process for FutureProcess<Fut>
where
    Fut: Future<Output = ()>,
{
    fn name(&self) -> &'static str {
        "FutureProcess"
    }
}

// NOTE: `ActorFuture` already catches panics for us.
impl<S, NA> Process for ActorFuture<S, NA>
where
    S: Supervisor<NA>,
    NA: NewActor,
    NA::RuntimeAccess: Clone,
{
    fn id(self: Pin<&Self>, _: ProcessId) -> ProcessId {
        ProcessId(self.pid())
    }

    fn name(&self) -> &'static str {
        NA::name()
    }
}

/// Data related to a process.
///
/// # Notes
///
/// `PartialEq` and `Eq` are implemented based on the id of the process
/// (`ProcessId`).
///
/// `PartialOrd` and `Ord` however are implemented based on runtime and
/// priority.
pub(crate) struct ProcessData<P: ?Sized> {
    priority: Priority,
    /// Fair runtime of the process, which is `actual runtime * priority`.
    fair_runtime: Duration,
    process: Pin<Box<P>>,
}

impl<P: ?Sized> ProcessData<P> {
    pub(crate) const fn new(priority: Priority, process: Pin<Box<P>>) -> ProcessData<P> {
        ProcessData {
            priority,
            fair_runtime: Duration::ZERO,
            process,
        }
    }

    #[cfg(test)]
    pub(crate) fn set_fair_runtime(&mut self, fair_runtime: Duration) {
        self.fair_runtime = fair_runtime;
    }
}

impl<P: Process + ?Sized> ProcessData<P> {
    /// Returns the process identifier, or pid for short.
    pub(crate) fn id(&self) -> ProcessId {
        let alternative = ProcessId(ptr::addr_of!(*self) as usize);
        self.process.as_ref().id(alternative)
    }

    /// Returns the name of the process.
    pub(crate) fn name(&self) -> &'static str {
        self.process.name()
    }

    /// Run the process.
    ///
    /// Returns the completion state of the process.
    pub(crate) fn run(mut self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> RunStats {
        let pid = self.as_ref().id();
        let name = self.process.name();
        trace!(pid = pid.0, name = name; "running process");

        let start = Instant::now();
        let result = self.process.as_mut().poll(ctx);
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

impl<P: Process + ?Sized> Eq for ProcessData<P> {}

impl<P: Process + ?Sized> PartialEq for ProcessData<P> {
    fn eq(&self, other: &Self) -> bool {
        Pin::new(self).id() == Pin::new(other).id()
    }
}

impl<P: Process + ?Sized> Ord for ProcessData<P> {
    fn cmp(&self, other: &Self) -> Ordering {
        (other.fair_runtime)
            .cmp(&(self.fair_runtime))
            .then_with(|| self.priority.cmp(&other.priority))
    }
}

impl<P: Process + ?Sized> PartialOrd for ProcessData<P> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[allow(clippy::missing_fields_in_debug)]
impl<P: Process + ?Sized> fmt::Debug for ProcessData<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Process")
            .field("id", &self.id())
            .field("name", &self.name())
            .field("priority", &self.priority)
            .field("fair_runtime", &self.fair_runtime)
            .finish()
    }
}
