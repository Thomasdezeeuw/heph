//! Module containing the process related types and implementations.

use std::cmp::Ordering;
use std::pin::Pin;
use std::task::{self, Poll};
use std::time::{Duration, Instant};
use std::{fmt, ptr};

use log::trace;

use crate::scheduler::{Priority, Schedule};
use crate::setup::scheduler::ProcessId;

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
    #[allow(clippy::struct_field_names)]
    process: Pin<Box<P>>,
}

impl<S: Schedule, P: crate::setup::scheduler::Process + ?Sized> Process<S, P> {
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
        let pid = ProcessId::new(ptr::from_ref(&**self).addr());
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
        let result = self.process.as_mut().poll(ctx);
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

impl<S, P: crate::setup::scheduler::Process + ?Sized> Process<S, P> {
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
impl<S: fmt::Debug, P: crate::setup::scheduler::Process + ?Sized> fmt::Debug for Process<S, P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Process")
            .field("id", &self.id())
            .field("scheduler_data", &self.scheduler_data)
            .field("name", &self.name())
            .finish()
    }
}
