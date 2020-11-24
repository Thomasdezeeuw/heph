//! Module containing the schedulers and related types.

use std::cmp::Ordering;
use std::fmt;
use std::mem::MaybeUninit;
use std::pin::Pin;
use std::time::{Duration, Instant};

use log::trace;

use crate::rt::process::{Process, ProcessId, ProcessResult};
use crate::RuntimeRef;

mod local;
mod priority;
mod shared;

#[cfg(test)]
mod tests;

pub(super) use local::LocalScheduler;
// Use in `test` module.
pub(crate) use shared::{Scheduler, SchedulerRef};

pub use priority::Priority;

/// Data related to a process.
///
/// # Notes
///
/// `PartialEq` and `Eq` are implemented based on the id of the process
/// (`ProcessId`).
///
/// `PartialOrd` and `Ord` however are implemented based on runtime and
/// priority.
pub(super) struct ProcessData<P: ?Sized> {
    priority: Priority,
    /// Fair runtime of the process, which is `actual runtime * priority`.
    fair_runtime: Duration,
    process: Pin<Box<P>>,
}

impl<P: ?Sized> ProcessData<P> {
    /// Returns the process identifier, or pid for short.
    fn id(self: Pin<&Self>) -> ProcessId {
        // Since the pid only job is to be unique we just use the pointer to
        // this structure as pid. This way we don't have to store any additional
        // pid in the structure itself or in the scheduler.
        #[allow(trivial_casts)]
        let ptr = unsafe { Pin::into_inner_unchecked(self) as *const _ as *const u8 };
        ProcessId(ptr as usize)
    }
}

impl<P: Process + ?Sized> ProcessData<P> {
    /// Run the process.
    ///
    /// Returns the completion state of the process.
    pub(super) fn run(mut self: Pin<&mut Self>, runtime_ref: &mut RuntimeRef) -> ProcessResult {
        let pid = self.as_ref().id();
        let name = self.process.name();
        trace!("running process: pid={}, name={}", pid, name);

        let start = Instant::now();
        let result = self.process.as_mut().run(runtime_ref, pid);
        let elapsed = start.elapsed();
        let fair_elapsed = elapsed * self.priority;
        self.fair_runtime += fair_elapsed;

        trace!(
            "finished running process: pid={}, name={}, elapsed_time={:?}, result={:?}",
            pid,
            name,
            elapsed,
            result
        );

        result
    }
}

impl<P: ?Sized> Eq for ProcessData<P> {}

impl<P: ?Sized> PartialEq for ProcessData<P> {
    fn eq(&self, other: &Self) -> bool {
        // FIXME: is this correct?
        Pin::new(self).id() == Pin::new(other).id()
    }
}

impl<P: ?Sized> Ord for ProcessData<P> {
    fn cmp(&self, other: &Self) -> Ordering {
        (other.fair_runtime)
            .cmp(&(self.fair_runtime))
            .then_with(|| self.priority.cmp(&other.priority))
    }
}

impl<P: ?Sized> PartialOrd for ProcessData<P> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<P: ?Sized> fmt::Debug for ProcessData<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Process")
            // FIXME: is this unsafe?
            .field("id", &Pin::new(self).id())
            .field("priority", &self.priority)
            .field("fair_runtime", &self.fair_runtime)
            .finish()
    }
}

/// A handle to add a process to the scheduler.
///
/// This allows the `ProcessId` to be determined before the process is actually
/// added. This is used in registering with the system poller.
pub(super) struct AddActor<I, P: ?Sized> {
    processes: I,
    /// Already allocated `ProcessData`, used to determine the `ProcessId`.
    alloc: Box<MaybeUninit<ProcessData<P>>>,
}

impl<I, P: ?Sized> AddActor<I, P> {
    /// Get the would be `ProcessId` for the process.
    pub(super) const fn pid(&self) -> ProcessId {
        #[allow(trivial_casts)]
        ProcessId(unsafe { &*self.alloc as *const _ as *const u8 as usize })
    }
}
