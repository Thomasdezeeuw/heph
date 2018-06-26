//! Module containing process related types and implementation.

use std::fmt;

use mio_st::event::EventedId;

use system::ActorSystemRef;

/// Process id, or pid, is an unique id for a process in an `ActorSystem`.
///
/// This id can only be created by [`ProcessIdGenerator`]. For convince this can
/// converted into an `EventedId` as used by `mio-st`.
///
/// [`ProcessIdGenerator`]: struct.ProcessIdGenerator.html
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ProcessId(pub(super) usize);

impl From<EventedId> for ProcessId {
    fn from(id: EventedId) -> ProcessId {
        ProcessId(id.0)
    }
}

impl Into<EventedId> for ProcessId {
    fn into(self) -> EventedId {
        EventedId(self.0)
    }
}

impl fmt::Display for ProcessId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// The trait that represents a process.
///
/// The main implementation is the `ActorProcess`, which is implementation of
/// this trait that revolves around an `Actor`.
pub trait Process: fmt::Debug {
    /// Run the process.
    ///
    /// If this function returns it is assumed that the process is:
    /// - done completely, i.e. it doesn't have to be run anymore, or
    /// - it would block, and itself made sure it's scheduled again at a later
    ///   point.
    fn run(&mut self, &mut ActorSystemRef) -> ProcessCompletion;
}

/// The result of running a `Process`.
#[must_use]
#[derive(Copy, Clone, Debug)]
pub enum ProcessCompletion {
    /// The process is complete.
    Complete,

    /// Process completion is pending, but for now no further process can be
    /// made.
    Pending,
}
