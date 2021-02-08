//! Module with the thread-local scheduler.
//!
//! Scheduler for the actors started with [`RuntimeRef::try_spawn_local`].
//!
//! [`RuntimeRef::try_spawn_local`]: crate::rt::RuntimeRef::try_spawn_local

use std::collections::BinaryHeap;
use std::fmt;
use std::mem::MaybeUninit;
use std::pin::Pin;

use inbox::Manager;
use log::trace;

use crate::actor::context;
use crate::rt::process::{self, ActorProcess, ProcessId};
use crate::{NewActor, Supervisor};

mod inactive;
mod priority;

#[cfg(test)]
mod tests;

use inactive::Inactive;

pub use priority::Priority;

type ProcessData = process::ProcessData<dyn process::Process>;

pub(crate) struct LocalScheduler {
    /// Processes that are ready to run.
    ready: BinaryHeap<Pin<Box<ProcessData>>>,
    /// Processes that are not ready to run.
    inactive: Inactive,
}

impl LocalScheduler {
    /// Create a new `LocalScheduler`.
    pub(crate) fn new() -> LocalScheduler {
        LocalScheduler {
            ready: BinaryHeap::new(),
            inactive: Inactive::empty(),
        }
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

    /// Mark the process, with `pid`, as ready to run.
    ///
    /// # Notes
    ///
    /// Calling this with an invalid or outdated `pid` will be silently ignored.
    pub(crate) fn mark_ready(&mut self, pid: ProcessId) {
        trace!("marking process as ready: pid={}", pid);
        if let Some(process) = self.inactive.remove(pid) {
            self.ready.push(process)
        }
    }

    /// Returns the next ready process.
    pub(crate) fn next_process(&mut self) -> Option<Pin<Box<ProcessData>>> {
        self.ready.pop()
    }

    /// Add back a process that was previously removed via
    /// [`LocalScheduler::next_process`].
    pub(crate) fn add_process(&mut self, process: Pin<Box<ProcessData>>) {
        self.inactive.add(process);
    }
}

impl fmt::Debug for LocalScheduler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LocalScheduler")
    }
}

/// A handle to add a process to the scheduler.
///
/// This allows the `ProcessId` to be determined before the process is actually
/// added. This is used in registering with the system poller.
pub(crate) struct AddActor<'s> {
    scheduler: &'s mut LocalScheduler,
    /// Already allocated `ProcessData`, used to determine the `ProcessId`.
    alloc: Box<MaybeUninit<ProcessData>>,
}

impl<'s> AddActor<'s> {
    /// Get the would be `ProcessId` for the process.
    pub(crate) const fn pid(&self) -> ProcessId {
        #[allow(trivial_casts)]
        ProcessId(unsafe { &*self.alloc as *const _ as *const u8 as usize })
    }

    /// Add a new inactive actor to the scheduler.
    pub(crate) fn add<S, NA>(
        self,
        priority: Priority,
        supervisor: S,
        new_actor: NA,
        actor: NA::Actor,
        inbox: Manager<NA::Message>,
        is_ready: bool,
    ) where
        S: Supervisor<NA> + 'static,
        NA: NewActor<Context = context::ThreadLocal> + 'static,
    {
        #[allow(trivial_casts)]
        debug_assert!(
            inactive::ok_ptr(self.alloc.as_ptr() as *const ()),
            "SKIP_BITS invalid"
        );
        let process = ProcessData::new(
            priority,
            Box::pin(ActorProcess::new(supervisor, new_actor, actor, inbox)),
        );
        let AddActor {
            scheduler,
            mut alloc,
        } = self;
        let process: Pin<_> = unsafe {
            let _ = alloc.write(process);
            // Safe because we write into the allocation above.
            alloc.assume_init().into()
        };
        if is_ready {
            scheduler.ready.push(process)
        } else {
            scheduler.inactive.add(process);
        }
    }
}
