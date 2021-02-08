//! Module with the thread-safe scheduler.
//!
//! Scheduler for the actors started with [`RuntimeRef::try_spawn`].
//!
//! [`RuntimeRef::try_spawn`]: crate::rt::RuntimeRef::try_spawn

use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use inbox::Manager;
use log::trace;

use crate::actor::{context, NewActor};
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
#[derive(Clone, Debug)]
pub(crate) struct Scheduler {
    shared: Arc<Shared>,
}

/// Internal of the `Scheduler`, shared with zero or more `WorkStealer`s.
#[derive(Debug)]
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

    /// Returns `true` if the schedule has any processes (in any state), `false`
    /// otherwise.
    pub(in crate::rt) fn has_process(&self) -> bool {
        let has_inactive = { self.shared.inactive.lock().unwrap().has_process() };
        has_inactive || self.has_ready_process()
    }

    /// Returns `true` if the schedule has any processes that are ready to run,
    /// `false` otherwise.
    pub(in crate::rt) fn has_ready_process(&self) -> bool {
        self.shared.ready.has_process()
    }

    /// Add a thread-safe actor to the scheduler.
    pub(in crate::rt) fn add_actor<'s>(
        &'s self,
    ) -> AddActor<&'s Scheduler, dyn Process + Send + Sync> {
        AddActor {
            processes: &self,
            alloc: Box::new_uninit(),
        }
    }

    /// Mark the process, with `pid`, as ready to run.
    ///
    /// # Notes
    ///
    /// Calling this with an invalid or outdated `pid` will be silently ignored.
    pub(in crate::rt) fn mark_ready(&mut self, pid: ProcessId) {
        trace!("marking process as ready: pid={}", pid);
        if let Some(process) = { self.shared.inactive.lock().unwrap().mark_ready(pid) } {
            // The process was in the `Inactive` list, so we move it to the run
            // queue.
            self.shared.ready.add(process)
        }
        // NOTE: if the process in currently not in the `Inactive` list it will
        // be marked as ready-to-run and `Scheduler::add_process` will add it to
        // the run queue once its done running.
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
        let pid = process.as_ref().id();

        trace!("adding back process: pid={}", pid);
        if let Some(process) = { self.shared.inactive.lock().unwrap().add(process) } {
            // If the process was marked as ready-to-run we need to add to the
            // run queue instead.
            self.shared.ready.add(process);
        }
    }

    /// Mark the `process` as complete.
    pub(in crate::rt) fn complete(&self, process: Pin<Box<ProcessData>>) {
        let pid = process.as_ref().id();
        trace!("removing process: pid={}", pid);

        // NOTE: we could leave a ready-to-run marker in the `Inactive` list,
        // but its not really worth it (locking other workers out) to remove it.

        drop(process);
    }
}

impl<'s> AddActor<&'s Scheduler, dyn Process + Send + Sync> {
    /// Add a new inactive thread-safe actor to the scheduler.
    pub(in crate::rt) fn add<S, NA>(
        self,
        priority: Priority,
        supervisor: S,
        new_actor: NA,
        actor: NA::Actor,
        inbox: Manager<NA::Message>,
        is_ready: bool,
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
            process: Box::pin(ActorProcess::new(supervisor, new_actor, actor, inbox)),
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

        if is_ready {
            processes.shared.ready.add(process);
        } else {
            processes.add_process(process)
        }
    }
}
