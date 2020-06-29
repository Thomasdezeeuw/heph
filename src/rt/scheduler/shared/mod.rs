//! Module with the thread-safe scheduler.
//!
//! Scheduler for the actors started with [`RuntimeRef::try_spawn`].
//!
//! [`RuntimeRef::try_spawn`]: crate::rt::RuntimeRef::try_spawn

use std::fmt;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use log::trace;
use parking_lot::Mutex;

use crate::actor::{context, NewActor};
use crate::inbox::{Inbox, InboxRef};
use crate::rt::process::{ActorProcess, Process, ProcessId};
use crate::rt::scheduler::{self, AddActor, Priority};
use crate::Supervisor;

mod inactive;
mod runqueue;

use inactive::Inactive;
use runqueue::RunQueue;

pub(super) type ProcessData = scheduler::ProcessData<dyn Process + Send + Sync>;

// # How the `Scheduler` works.
//
// There are two components to the scheduler:
//
// * `RunQueue`: holds the processes that are ready to run.
// * `Inactive`: holds the inactive processes.
//
// Both components are shared between `Scheduler` and zero or more
// `WorkStealer`s. This `WorkStealer` can be used to steal processes in the
// ready state from this scheduler. This is used by other workers threads to
// steal work for themselves in an effort to prevent an in-balance in the
// workload.
//
//
// ## Process states
//
// Processes can be in one of the following states:
//
// * Inactive: default state of an process, its located in `Scheduler.inactive`.
// * Ready: process is ready to run (after they are marked as such, see
//   `Scheduler::mark_ready`), located in `Scheduler.ready`.
// * Running: process is being run, located on the stack on the worker thread
//   that is running it.
// * Stopped: final state of a process, at this point its deallocated and its
//   resources cleaned up.

/// The thread-safe scheduler, responsible for scheduling processes.
pub(crate) struct Scheduler {
    shared: Arc<Shared>,
}

/// Internal of the `Scheduler`, shared with zero or more `WorkStealer`s.
struct Shared {
    /// Processes that are ready to run.
    ready: RunQueue,
    /// Inactive processes that are not ready to run.
    inactive: Mutex<Inactive>,
}

impl Scheduler {
    /// Create a new `Scheduler` and accompanying reference.
    pub(crate) fn new() -> Scheduler {
        let shared = Arc::new(Shared {
            ready: RunQueue::empty(),
            inactive: Mutex::new(Inactive::empty()),
        });
        Scheduler { shared }
    }

    /// Mark the process, with `pid`, as ready to run.
    ///
    /// # Notes
    ///
    /// Calling this with an invalid or outdated `pid` will be silently ignored.
    pub(in crate::rt) fn mark_ready(&mut self, pid: ProcessId) {
        trace!("marking process as ready: pid={}", pid);
        if let Some(process) = self.shared.inactive.lock().remove(pid) {
            self.shared.ready.add(process)
        }
    }

    /// Create a [`SchedulerRef`] referring to this scheduler.
    pub(crate) fn create_ref(&self) -> SchedulerRef {
        SchedulerRef {
            shared: self.shared.clone(),
        }
    }
}

impl fmt::Debug for Scheduler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Scheduler")
    }
}

/// Handle to a [`Scheduler`].
#[derive(Clone)]
pub(crate) struct SchedulerRef {
    shared: Arc<Shared>,
}

impl SchedulerRef {
    /// Returns `true` if the schedule has any processes (in any state), `false`
    /// otherwise.
    pub(in crate::rt) fn has_process(&self) -> bool {
        !self.shared.inactive.lock().has_process() || self.has_ready_process()
    }

    /// Returns `true` if the schedule has any processes that are ready to run,
    /// `false` otherwise.
    pub(in crate::rt) fn has_ready_process(&self) -> bool {
        self.shared.ready.has_process()
    }

    /// Add a thread-safe actor to the scheduler.
    pub(in crate::rt) fn add_actor<'s>(
        &'s self,
    ) -> AddActor<&'s Mutex<Inactive>, dyn Process + Send + Sync> {
        AddActor {
            processes: &self.shared.inactive,
            alloc: Box::new_uninit(),
        }
    }

    /// Attempts to steal a process.
    ///
    /// Returns:
    /// * `Ok(Some(..))` if a process was successfully stolen.
    /// * `Ok(None)` if no processes are available to run.
    pub(in crate::rt) fn try_steal(&self) -> Option<Pin<Box<ProcessData>>> {
        self.shared.ready.remove()
    }

    /// Add back a process that was previously removed via
    /// [`SchedulerRef::try_steal`].
    pub(in crate::rt) fn add_process(&self, process: Pin<Box<ProcessData>>) {
        self.shared.inactive.lock().add(process)
    }
}

impl fmt::Debug for SchedulerRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SchedulerRef")
    }
}

impl<'s> AddActor<&'s Mutex<Inactive>, dyn Process + Send + Sync> {
    /// Add a new inactive thread-safe actor to the scheduler.
    pub(in crate::rt) fn add<S, NA>(
        self,
        priority: Priority,
        supervisor: S,
        new_actor: NA,
        actor: NA::Actor,
        inbox: Inbox<NA::Message>,
        inbox_ref: InboxRef<NA::Message>,
    ) where
        S: Supervisor<NA> + Send + Sync + 'static,
        NA: NewActor<Context = context::ThreadSafe> + Send + Sync + 'static,
        NA::Actor: Send + Sync + 'static,
        NA::Message: Send,
    {
        #[allow(trivial_casts)]
        debug_assert!(
            inactive::ok_ptr(self.alloc.as_ptr() as *const ()),
            "SKIP_BITS invalid"
        );

        let process = ProcessData {
            priority,
            fair_runtime: Duration::from_nanos(0),
            process: Box::pin(ActorProcess::new(
                supervisor, new_actor, actor, inbox, inbox_ref,
            )),
        };
        let AddActor {
            processes,
            mut alloc,
        } = self;
        let process: Pin<_> = unsafe {
            let _ = alloc.write(process);
            // Safe because we write into the allocation above.
            alloc.assume_init().into()
        };
        processes.lock().add(process)
    }
}
