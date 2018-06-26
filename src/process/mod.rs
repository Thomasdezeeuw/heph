//! Module containing the `Process` trait, related types and implementations.

use std::fmt;

use mio_st::event::EventedId;

use system::ActorSystemRef;

mod actor;
mod initiator;

pub use self::actor::{ActorProcess, ActorRef};
pub use self::initiator::InitiatorProcess;

/// Process id, or pid for short, is an unique id for a process in an
/// `ActorSystem`.
///
/// This can only be created by the [`Scheduler`] and should be seen as an
/// opaque type for the rest of the crate. For convince this can converted from
/// and into an `EventedId` as used by `mio-st`.
///
/// [`Scheduler`]: ../scheduler/struct.Scheduler.html
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ProcessId(pub usize);

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
/// This currently has two implementations;
/// - the `ActorProcess`, which wraps an `Actor` to implement this trait, and
/// - the `InitiatorProcess`, which wraps an `Initiator`.
pub trait Process: fmt::Debug {
    /// Run the process.
    fn run(&mut self, &mut ActorSystemRef) -> ProcessCompletion;
}

/// The result of running a `Process`.
#[must_use]
#[derive(Copy, Clone, Debug)]
pub enum ProcessCompletion {
    /// The process is complete.
    Complete,

    /// Process completion is pending, but for now no further process can be
    /// made without blocking. The process itself is responsible for scheduling
    /// itself again.
    Pending,
}
