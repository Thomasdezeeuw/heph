//! Module containing the `Process` trait, related types and implementations.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::pin::Pin;

use mio::Token;

use crate::RuntimeRef;

mod actor;

#[cfg(test)]
mod tests;

pub use actor::ActorProcess;

/// Process id, or pid for short, is an identifier for a process in an
/// [`Runtime`].
///
/// This can only be created by the [`Scheduler`] and should be seen as an
/// opaque type for the rest of the crate. For convince this can converted from
/// and into an [`Token`] as used by Mio.
///
/// [`Runtime`]: crate::Runtime
/// [`Scheduler`]: crate::rt::scheduler::Scheduler
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(transparent)]
pub struct ProcessId(pub usize);

impl From<Token> for ProcessId {
    fn from(id: Token) -> ProcessId {
        ProcessId(id.0)
    }
}

impl Into<Token> for ProcessId {
    fn into(self) -> Token {
        Token(self.0)
    }
}

impl Hash for ProcessId {
    fn hash<H>(&self, state: &mut H)
    where
        H: Hasher,
    {
        self.0.hash(state)
    }
}

impl fmt::Display for ProcessId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// The trait that represents a process.
///
/// This currently has a single implementation:
/// - `ActorProcess`, which wraps an `Actor` to implement this trait.
pub trait Process {
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
pub enum ProcessResult {
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
