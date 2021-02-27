//! Module containing the `Process` trait, related types and implementations.

use std::cmp::Ordering;
use std::fmt;
use std::pin::Pin;
use std::time::{Duration, Instant};

use log::trace;
use mio::Token;

use crate::rt::options::Priority;
use crate::rt::RuntimeRef;

mod actor;
#[cfg(test)]
mod tests;

pub(crate) use actor::ActorProcess;

/// Process id, or pid for short, is an identifier for a process in an
/// [`Runtime`].
///
/// This can only be created by one of the schedulers and should be seen as an
/// opaque type for the rest of the crate. For convince this can converted from
/// and into an [`Token`] as used by Mio.
///
/// [`Runtime`]: crate::Runtime
// NOTE: public because it used in the `RuntimeAccess` trait.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Ord, PartialOrd)]
#[repr(transparent)]
pub struct ProcessId(pub(crate) usize);

impl From<Token> for ProcessId {
    fn from(id: Token) -> ProcessId {
        ProcessId(id.0)
    }
}

impl From<ProcessId> for Token {
    fn from(id: ProcessId) -> Token {
        Token(id.0)
    }
}

impl fmt::Debug for ProcessId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for ProcessId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The trait that represents a process.
///
/// This currently has a single implementation:
/// - `ActorProcess`, which wraps an `Actor` to implement this trait.
pub(crate) trait Process {
    /// Return the name of this process, used in logging.
    fn name(&self) -> &'static str;

    /// Run the process.
    ///
    /// Once the process returns `ProcessResult::Complete` it will be removed
    /// from the scheduler and will no longer run.
    ///
    /// If it returns `ProcessResult::Pending` it will be considered inactive
    /// and the process itself must make sure its gets scheduled again.
    fn run(self: Pin<&mut Self>, runtime_ref: &mut RuntimeRef, pid: ProcessId) -> ProcessResult;
}

/// The result of running a [`Process`].
///
/// See [`Process::run`].
#[must_use]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProcessResult {
    /// The process is complete.
    ///
    /// Similar to [`Poll::Ready`].
    ///
    /// [`Poll::Ready`]: std::task::Poll::Ready
    Complete,
    /// Process completion is pending, but for now no further progress can be
    /// made without blocking. The process itself is responsible for scheduling
    /// itself again.
    ///
    /// Similar to [`Poll::Pending`].
    ///
    /// [`Poll::Pending`]: std::task::Poll::Pending
    Pending,
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

    /// Returns the process identifier, or pid for short.
    pub(crate) fn id(self: Pin<&Self>) -> ProcessId {
        // Since the pid only job is to be unique we just use the pointer to
        // this structure as pid. This way we don't have to store any additional
        // pid in the structure itself or in the scheduler.
        #[allow(trivial_casts)]
        let ptr = unsafe { Pin::into_inner_unchecked(self) as *const _ as *const u8 };
        ProcessId(ptr as usize)
    }
}

impl<P: Process + ?Sized> ProcessData<P> {
    /// Returns the name of the process.
    pub(crate) fn name(self: Pin<&Self>) -> &'static str {
        self.process.name()
    }

    /// Run the process.
    ///
    /// Returns the completion state of the process.
    pub(crate) fn run(mut self: Pin<&mut Self>, runtime_ref: &mut RuntimeRef) -> ProcessResult {
        let pid = self.as_ref().id();
        let name = self.process.name();
        trace!("running process: pid={}, name={}", pid, name);

        let start = Instant::now();
        let result = self.process.as_mut().run(runtime_ref, pid);
        let elapsed = start.elapsed();
        let fair_elapsed = elapsed * self.priority;
        self.fair_runtime += fair_elapsed;

        trace!(
            "finished running process: pid={}, name={}, elapsed_time={:?}, result={:?}",
            pid,
            name,
            elapsed,
            result
        );

        result
    }
}

impl<P: ?Sized> Eq for ProcessData<P> {}

impl<P: ?Sized> PartialEq for ProcessData<P> {
    fn eq(&self, other: &Self) -> bool {
        // FIXME: is this correct?
        Pin::new(self).id() == Pin::new(other).id()
    }
}

impl<P: ?Sized> Ord for ProcessData<P> {
    fn cmp(&self, other: &Self) -> Ordering {
        (other.fair_runtime)
            .cmp(&(self.fair_runtime))
            .then_with(|| self.priority.cmp(&other.priority))
    }
}

impl<P: ?Sized> PartialOrd for ProcessData<P> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<P: ?Sized> fmt::Debug for ProcessData<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Process")
            // FIXME: is this unsafe?
            .field("id", &Pin::new(self).id())
            .field("priority", &self.priority)
            .field("fair_runtime", &self.fair_runtime)
            .finish()
    }
}
