//! Scheduler implementations.

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::time::{Duration, Instant};

use log::trace;

use crate::spawn::options::Priority;

mod inactive;
pub(crate) mod process;
pub(crate) mod shared;
#[cfg(test)]
mod tests;

use inactive::Inactive;
pub(crate) use process::ProcessId;

type Process = process::Process<dyn process::Run>;

#[derive(Debug)]
pub(crate) struct Scheduler {
    /// Processes that are ready to run.
    ready: BinaryHeap<Pin<Box<Process>>>,
    /// Processes that are not ready to run.
    inactive: Inactive,
}

impl Scheduler {
    /// Create a new `Scheduler`.
    pub(crate) fn new() -> Scheduler {
        Scheduler {
            ready: BinaryHeap::new(),
            inactive: Inactive::empty(),
        }
    }

    /// Returns the number of processes ready to run.
    pub(crate) fn ready(&self) -> usize {
        self.ready.len()
    }

    /// Returns the number of inactive processes.
    pub(crate) const fn inactive(&self) -> usize {
        self.inactive.len()
    }

    /// Returns `true` if the scheduler has any user processes (in any state),
    /// `false` otherwise. This ignore system processes.
    pub(crate) fn has_user_process(&self) -> bool {
        self.has_ready_process() || self.inactive.has_user_process()
    }

    /// Returns `true` if the scheduler has any processes that are ready to run,
    /// `false` otherwise.
    pub(crate) fn has_ready_process(&self) -> bool {
        !self.ready.is_empty()
    }

    /// Add a new proces to the scheduler.
    pub(crate) fn add_new_process<P>(&mut self, priority: Priority, process: P) -> ProcessId
    where
        P: process::Run + 'static,
    {
        let process = Box::pin(Process::new(priority, Box::pin(process)));
        let pid = process.id();
        self.ready.push(process);
        pid
    }

    /// Mark the process, with `pid`, as ready to run.
    ///
    /// # Notes
    ///
    /// Calling this with an invalid or outdated `pid` will be silently ignored.
    pub(crate) fn mark_ready(&mut self, pid: ProcessId) {
        trace!(pid = pid.0; "marking process as ready");
        if let Some(process) = self.inactive.remove(pid) {
            self.ready.push(process);
        }
    }

    /// Returns the next ready process.
    pub(crate) fn next_process(&mut self) -> Option<Pin<Box<Process>>> {
        self.ready.pop()
    }

    /// Add back a process that was previously removed via
    /// [`Scheduler::next_process`].
    pub(crate) fn add_back_process(&mut self, process: Pin<Box<Process>>) {
        let pid = process.id();
        trace!(pid = pid.0; "adding back process");
        self.inactive.add(process);
    }

    /// Mark `process` as complete, removing it from the scheduler.
    #[allow(clippy::unused_self)]
    pub(crate) fn complete(&self, process: Pin<Box<Process>>) {
        let pid = process.as_ref().id();
        trace!(pid = pid.0; "removing process");
        // Don't want to panic when dropping the process.
        drop(catch_unwind(AssertUnwindSafe(move || drop(process))));
    }
}

/// Scheduling implementation.
trait Schedule {
    /// Data stored per process for scheduling.
    type ProcessData: ProcessData;

    /// Determine if the `lhs` or `rhs` should run first.
    fn order(lhs: &Self::ProcessData, rhs: &Self::ProcessData) -> Ordering;
}

/// Scheduler data stored per process used for scheduling purposes.
trait ProcessData {
    /// Create new data.
    fn new(priority: Priority) -> Self;

    /// Update the process data with the latest run information.
    ///
    /// Arguments:
    ///  * `start`: time at which the latest run started.
    ///  * `end`: time at which the latest run ended.
    ///  * `elapsed`: `end - start`.
    fn update(&mut self, start: Instant, end: Instant, elapsed: Duration);
}
