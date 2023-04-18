//! Module with the thread-local scheduler.
//!
//! Scheduler for the actors started with [`RuntimeRef::try_spawn_local`].
//!
//! [`RuntimeRef::try_spawn_local`]: crate::RuntimeRef::try_spawn_local

use std::collections::BinaryHeap;
use std::future::Future;
use std::mem::MaybeUninit;
use std::pin::Pin;

use log::{debug, trace};

use crate::process::{self, FutureProcess, ProcessId};
use crate::spawn::options::Priority;
use crate::{ptr_as_usize, ThreadLocal};

mod inactive;
#[cfg(test)]
mod tests;

use inactive::Inactive;

type ProcessData = process::ProcessData<dyn process::Process>;

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

    /// Add an actor to the scheduler.
    pub(crate) fn add_actor<'s>(&'s mut self) -> AddActor<'s> {
        AddActor {
            scheduler: self,
            alloc: Box::new_uninit(),
        }
    }

    pub(crate) fn add_future<Fut>(&mut self, future: Fut, priority: Priority)
    where
        Fut: Future<Output = ()> + 'static,
    {
        let process = Box::pin(ProcessData::new(
            priority,
            Box::pin(FutureProcess::<Fut, ThreadLocal>::new(future)),
        ));
        debug!(pid = process.as_ref().id().0; "spawning thread-local future");
        self.ready.push(process);
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
    pub(crate) fn add_process(&mut self, process: Pin<Box<ProcessData>>) {
        self.inactive.add(process);
    }
}

/// A handle to add a process to the scheduler.
///
/// This allows the `ProcessId` to be determined before the process is actually
/// added. This is used in registering with the system poller.
pub(crate) struct AddActor<'s> {
    scheduler: &'s mut Scheduler,
    /// Already allocated `ProcessData`, used to determine the `ProcessId`.
    alloc: Box<MaybeUninit<ProcessData>>,
}

impl<'s> AddActor<'s> {
    /// Get the would be `ProcessId` for the process.
    pub(crate) const fn pid(&self) -> ProcessId {
        #[allow(clippy::borrow_as_ptr)]
        ProcessId(ptr_as_usize(&*self.alloc as *const _))
    }

    /// Add a new inactive actor to the scheduler.
    pub(crate) fn add<Fut>(self, future: Fut, priority: Priority)
    where
        Fut: Future<Output = ()> + 'static,
    {
        debug_assert!(
            inactive::ok_ptr(self.alloc.as_ptr().cast::<()>()),
            "SKIP_BITS invalid"
        );
        let process = ProcessData::new(
            priority,
            Box::pin(FutureProcess::<Fut, ThreadLocal>::new(future)),
        );
        let AddActor {
            scheduler,
            mut alloc,
        } = self;
        let process: Pin<_> = unsafe {
            _ = alloc.write(process);
            // Safe because we write into the allocation above.
            alloc.assume_init().into()
        };
        scheduler.ready.push(process);
    }
}
