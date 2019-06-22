//! Module containing the `Process` trait, related types and implementations.

use std::fmt;
use std::pin::Pin;

use gaea::event;

use crate::system::ActorSystemRef;

mod actor;

#[cfg(test)]
mod tests;

pub use actor::ActorProcess;

/// Process id, or pid for short, is an identifier for a process in an
/// [`ActorSystem`].
///
/// This can only be created by the [`Scheduler`] and should be seen as an
/// opaque type for the rest of the crate. For convince this can converted from
/// and into an [`event::Id`] as used by Gaea.
///
/// [`ActorSystem`]: crate::system::ActorSystem
/// [`Scheduler`]: crate::system::scheduler::Scheduler
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(transparent)]
pub struct ProcessId(pub u32);

impl From<event::Id> for ProcessId {
    fn from(id: event::Id) -> ProcessId {
        ProcessId(id.0 as u32)
    }
}

impl Into<event::Id> for ProcessId {
    fn into(self) -> event::Id {
        event::Id(self.0 as usize)
    }
}

impl fmt::Display for ProcessId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// The trait that represents a process.
///
/// This currently has a single implementations;
/// - the `ActorProcess`, which wraps an `Actor` to implement this trait.
pub trait Process {
    /// Run the process.
    ///
    /// Once the process returns `ProcessResult::Complete` it will be removed
    /// from the system and no longer run.
    ///
    /// If it returns `ProcessResult::Pending` it will be considered inactive
    /// and the process itself must make sure its gets scheduled again.
    fn run(self: Pin<&mut Self>, system_ref: &mut ActorSystemRef, pid: ProcessId) -> ProcessResult;
}

/// The result of running a `Process`.
///
/// See [`Process::run`].
#[must_use]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ProcessResult {
    /// The process is complete.
    Complete,
    /// Process completion is pending, but for now no further progress can be
    /// made without blocking. The process itself is responsible for scheduling
    /// itself again.
    Pending,
}
