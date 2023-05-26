//! Thread-safe version of `Scheduler`.

use std::pin::Pin;

use log::trace;

use crate::process::{Process, ProcessId};
use crate::spawn::options::Priority;

mod inactive;
mod runqueue;
#[cfg(test)]
mod tests;

use inactive::Inactive;
use runqueue::RunQueue;

pub(crate) type ProcessData = crate::process::ProcessData<dyn Process + Send + Sync>;

/// The thread-safe scheduler, responsible for scheduling processes that can run
/// one any of the worker threads, e.g. thread-safe actors.
///
/// # How the scheduler works
///
/// There are two components to the scheduler:
///
/// * [`RunQueue`]: holds the processes that are ready to run.
/// * [`Inactive`]: holds the inactive processes.
///
/// All threads have access to both components to they can mark processes as
/// ready to run, e.g. in the waking mechanism, and allows worker threads to run
/// a process.
///
/// ## Process states
///
/// Processes can be in one of the following states:
///
/// * Inactive: process can't make progress, its located in
///   [`Scheduler::inactive`].
/// * Ready: process is ready to run (after they are marked as such, see
///   [`Scheduler::mark_ready`]), located in [`Scheduler::ready`].
/// * Running: process is being run, located on the stack on the worker thread
///   that is running it.
/// * Stopped: final state of a process, at this point its deallocated and its
///   resources cleaned up.
///
/// ## Adding processes
///
/// Adding new processes can be done using [`Scheduler::add_new_process`]. It
/// accepts a callback function to get access to the PID before the process is
/// actually added.
///
/// ## Marking a process as ready to run
///
/// Marking a process as ready to run is done by calling
/// [`Scheduler::mark_ready`], this move the actor from the [`Inactive`] list to
/// the [`RunQueue`].
///
/// If the process is not found in the [`Inactive`] a marker is placed in its
/// place in the list. This marker ensures that the process is marked as ready
/// to run the next time its added back to the scheduler. This is required to
/// overcome the race between the coordinator thread marking processes as ready
/// and worker threads running the processes, during which it might trigger (OS)
/// resources to become ready.
///
/// ## Running a process
///
/// A worker thread can by first removing a process from the `Scheduler` by
/// calling [`Scheduler::remove`]. The scheduler will check if the [`RunQueue`]
/// is non-empty and returns the highest priority process that is ready to run.
///
/// If `remove` returns `Some(process)` the process must be run. Depending on
/// the result of the process it should be added back the schduler using
/// [`Scheduler::add_back_process`], adding it back to the [`Inactive`] list, or
/// marked as completed using [`Scheduler::complete`], which cleans up any
/// resources assiociated with the process.
///
/// If the process was marked as ready to run while it was running, see the
/// section above, it will not be added to the [`Inactive`] list but instead be
/// moved to the [`RunQueue`] again.
#[derive(Debug)]
pub(crate) struct Scheduler {
    /// Processes that are ready to run.
    ready: RunQueue,
    /// Inactive processes that are not ready to run.
    inactive: Inactive,
}

impl Scheduler {
    /// Create a new `Scheduler`.
    pub(crate) const fn new() -> Scheduler {
        Scheduler {
            ready: RunQueue::empty(),
            inactive: Inactive::empty(),
        }
    }

    /// Returns the number of processes ready to run.
    pub(crate) fn ready(&self) -> usize {
        self.ready.len()
    }

    /// Returns the number of inactive processes.
    pub(crate) fn inactive(&self) -> usize {
        self.inactive.len()
    }

    /// Returns `true` if the scheduler has any processes (in any state),
    /// `false` otherwise.
    ///
    /// # Notes
    ///
    /// Once this function returns the value could already be outdated.
    pub(crate) fn has_process(&self) -> bool {
        let has_inactive = self.inactive.has_process();
        has_inactive || self.has_ready_process()
    }

    /// Returns `true` if the scheduler has any processes that are ready to run,
    /// `false` otherwise.
    ///
    /// # Notes
    ///
    /// Once this function returns the value could already be outdated.
    pub(crate) fn has_ready_process(&self) -> bool {
        self.ready.has_process()
    }

    /// Add a new proces to the scheduler.
    pub(crate) fn add_new_process<P>(&self, priority: Priority, process: P) -> ProcessId
    where
        P: Process + Send + Sync + 'static,
    {
        let process = Box::pin(ProcessData::new(priority, Box::pin(process)));
        let pid = process.as_ref().id();
        self.ready.add(process);
        pid
    }

    /// Mark the process, with `pid`, as ready to run.
    ///
    /// # Notes
    ///
    /// Calling this with an invalid or outdated `pid` will be silently ignored.
    pub(crate) fn mark_ready(&self, pid: ProcessId) {
        trace!(pid = pid.0; "marking process as ready");
        self.inactive.mark_ready(pid, &self.ready);
        // NOTE: if the process in currently not in the `Inactive` list it will
        // be marked as ready-to-run and `Scheduler::add_back_process` will add it to
        // the run queue once its done running.
    }

    /// Attempts to remove a process.
    ///
    /// Returns `Ok(Some(..))` if a process was successfully removed or
    /// `Ok(None)` if no processes are available to run.
    pub(crate) fn remove(&self) -> Option<Pin<Box<ProcessData>>> {
        self.ready.remove()
    }

    /// Add back a process that was previously removed via
    /// [`Scheduler::remove`] and add it to the inactive list.
    pub(crate) fn add_back_process(&self, process: Pin<Box<ProcessData>>) {
        let pid = process.as_ref().id();
        trace!(pid = pid.0; "adding back process");
        self.inactive.add(process, &self.ready);
    }

    /// Mark `process` as complete, removing it from the scheduler.
    pub(crate) fn complete(&self, process: Pin<Box<ProcessData>>) {
        let pid = process.as_ref().id();
        trace!(pid = pid.0; "removing process");
        self.inactive.complete(process);
    }
}
