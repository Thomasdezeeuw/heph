//! Module containing the process related types and implementations.

use std::cmp::Ordering;
use std::pin::Pin;
use std::task::{self, Poll};
use std::{fmt, ptr};

use crate::scheduler::{Priority, RunStats, Schedule, Task};
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
pub struct Process<S, T: ?Sized> {
    id: ProcessId,
    scheduler_data: S,
    task: Pin<Box<T>>,
}

impl<S: Schedule, T: Task + ?Sized> Process<S, T> {
    /// Create a new process container.
    pub(crate) fn new(id: ProcessId, priority: Priority, task: Pin<Box<T>>) -> Process<S, T> {
        Process {
            id,
            scheduler_data: S::new(priority),
            task,
        }
    }

    pub(crate) fn update(self: Pin<&mut Self>, stats: &RunStats) {
        unsafe { self.get_unchecked_mut().scheduler_data.update(stats) };
    }

    // TODO: remove
    pub(crate) fn set_id(self: &mut Pin<Box<Self>>) {
        let pid = ProcessId::new(ptr::from_ref(&**self).addr());
        unsafe { self.as_mut().get_unchecked_mut().id = pid };
    }
}

impl<S: Schedule, T: Task + ?Sized> Task for Process<S, T> {
    fn name(&self) -> &'static str {
        self.task.name()
    }
}

impl<S: Schedule, T: Task + ?Sized> crate::setup::scheduler::Process for Process<S, T> {
    fn id(&self) -> ProcessId {
        self.id
    }
}

impl<S: Schedule, T: Task + ?Sized> Future for Process<S, T> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<()> {
        // SAFETY: not moving the future.
        unsafe { Pin::map_unchecked_mut(self.as_mut(), |s| &mut s.task) }.poll(ctx)
    }
}

impl<S, T: ?Sized> Process<S, T> {
    /// Returns the process identifier, or pid for short.
    pub(crate) fn id(&self) -> ProcessId {
        self.id
    }

    #[cfg(test)]
    pub(crate) fn scheduler_data(&mut self) -> &mut S {
        &mut self.scheduler_data
    }
}

impl<S, T: Task + ?Sized> Process<S, T> {
    /// Returns the name of the process.
    pub(crate) fn name(&self) -> &'static str {
        self.task.name()
    }
}

impl<S, T: ?Sized> Eq for Process<S, T> {}

impl<S, T: ?Sized> PartialEq for Process<S, T> {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
    }
}

impl<S: Schedule, T: ?Sized> Ord for Process<S, T> {
    fn cmp(&self, other: &Self) -> Ordering {
        S::order(&self.scheduler_data, &other.scheduler_data)
    }
}

impl<S: Schedule, T: ?Sized> PartialOrd for Process<S, T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[allow(clippy::missing_fields_in_debug)]
impl<S: fmt::Debug, T: Task + ?Sized> fmt::Debug for Process<S, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Process")
            .field("id", &self.id())
            .field("scheduler_data", &self.scheduler_data)
            .field("name", &self.name())
            .finish()
    }
}
