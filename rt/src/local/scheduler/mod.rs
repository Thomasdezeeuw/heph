//! Thread-local scheduler.

use std::collections::BinaryHeap;
use std::mem::MaybeUninit;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;

use log::trace;

use crate::process::{self, Process, ProcessId};
use crate::ptr_as_usize;
use crate::spawn::options::Priority;

mod inactive;
#[cfg(test)]
mod tests;

use inactive::Inactive;

type ProcessData = process::ProcessData<dyn Process>;

#[derive(Debug)]
pub(crate) struct Scheduler {
    /// Processes that are ready to run.
    ready: BinaryHeap<Pin<Box<ProcessData>>>,
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

    /// Returns `true` if the scheduler has any processes (in any state),
    /// `false` otherwise.
    pub(crate) fn has_process(&self) -> bool {
        self.inactive.has_process() || self.has_ready_process()
    }

    /// Returns `true` if the scheduler has any processes that are ready to run,
    /// `false` otherwise.
    pub(crate) fn has_ready_process(&self) -> bool {
        !self.ready.is_empty()
    }

    /// Add a new proces to the scheduler.
    pub(crate) fn add_new_process<F, P, T, E>(
        &mut self,
        priority: Priority,
        setup: F,
    ) -> Result<T, E>
    where
        F: FnOnce(ProcessId) -> Result<(P, T), E>,
        P: Process + 'static,
    {
        // Allocate some memory for the process.
        let mut alloc: Box<MaybeUninit<ProcessData>> = Box::new_uninit();
        debug_assert!(inactive::ok_ptr(alloc.as_ptr().cast()), "SKIP_BITS invalid");
        // Based on the allocation we can determine its process id.
        let pid = ProcessId(ptr_as_usize(alloc.as_ptr()));
        // Let the caller create the actual process (using the pid).
        let (process, ret) = setup(pid)?;
        let process = ProcessData::new(priority, Box::pin(process));
        // SAFETY: we write the processes and then safetly assume it's initialised.
        let process = unsafe {
            _ = alloc.write(process);
            Pin::from(alloc.assume_init())
        };
        // Finally add it to ready queue.
        self.ready.push(process);
        Ok(ret)
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
    pub(crate) fn next_process(&mut self) -> Option<Pin<Box<ProcessData>>> {
        self.ready.pop()
    }

    /// Add back a process that was previously removed via
    /// [`Scheduler::next_process`].
    pub(crate) fn add_back_process(&mut self, process: Pin<Box<ProcessData>>) {
        let pid = process.as_ref().id();
        trace!(pid = pid.0; "adding back process");
        self.inactive.add(process);
    }

    /// Mark `process` as complete, removing it from the scheduler.
    pub(crate) fn complete(&self, process: Pin<Box<ProcessData>>) {
        let pid = process.as_ref().id();
        trace!(pid = pid.0; "removing process");
        // Don't want to panic when dropping the process.
        drop(catch_unwind(AssertUnwindSafe(move || drop(process))));
    }
}
