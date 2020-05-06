//! Module with the thread-safe scheduler.
//!
//! Scheduler for the actors started with [`RuntimeRef::try_spawn`].
//!
//! [`RuntimeRef::try_spawn`]: crate::rt::RuntimeRef::try_spawn

use std::fmt;
use std::ops::Deref;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use log::trace;
use parking_lot::{Mutex, RwLock};

use crate::actor::{context, NewActor};
use crate::inbox::{Inbox, InboxRef};
use crate::rt::process::{ActorProcess, ProcessId};
use crate::rt::scheduler::{AddActor, Priority, ProcessData};
use crate::Supervisor;

mod inactive;
mod runqueue;

use inactive::Inactive;
use runqueue::RunQueue;

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
// The `RunQueue` is behind a `RwLock` to enable a fast path for the owner of
// the run queue if no other thread is attempting to access it at the same time.
// To ensure the thread that owns the run queue is never blocked only the
// `Scheduler` can request mutable access, the `WorkStealer` may not. The
// `WorkStealer` can only request immutable access, this way other
// `WorkStealer`s and the `Scheduler` can continue to operate on the run queue
// concurrently (albeit using the slow path), ensure the thread that owns the
// run queue isn't blocked.
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
    ///
    /// # Notes
    ///
    /// This is shared between one `Scheduler` and zero or more `WorkStealer`s,
    /// which can steal ready to run processes from this scheduler.
    ///
    /// The `scheduler` may only lock using the following two methods:
    /// * `try_write`: attempts to get unique access to the processes.
    /// * `try_read`: if `try_write` fails another thread has access to it via a
    ///   `WorkStealer`. However `WorkStealer` may only use the read lock (see
    ///   below), meaning that this owner of the scheduler (this struct) can
    ///   always get access to the processes, with an optimisation if its the
    ///   only one with access.
    ///
    /// A `WorkStealer` may only lock using the `try_read` method. **No other
    /// methods are allowed to be used by the `WorkStealer`**.
    ready: RwLock<RunQueue>,
    /// Inactive processes that are not ready to run.
    inactive: Mutex<Inactive>,
}

impl Scheduler {
    /// Create a new `Scheduler` and accompanying reference.
    pub(crate) fn new() -> Scheduler {
        let shared = Arc::new(Shared {
            ready: RwLock::new(RunQueue::empty()),
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
            match self.shared.ready.try_write() {
                // Fast path.
                Some(mut run_queue) => run_queue.add_mut(process),
                // Slow path.
                None => self.run_queue(|run_queue| run_queue.add(process)),
            }
        }
    }

    /// Get immutable access to the run queue.
    fn run_queue<F, O>(&self, f: F) -> O
    where
        F: FnOnce(&RunQueue) -> O,
    {
        match self.shared.ready.try_read() {
            Some(run_queue) => f(run_queue.deref()),
            // This is impossible as `Scheduler` is the only object that can use
            // the `try_write` method, to which we have a unique reference, thus
            // `try_read` will never fail.
            None => unreachable!("Scheduler can't get access to RunQueue"),
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
        self.shared.ready.read().has_process()
    }

    /// Add a thread-safe actor to the scheduler.
    pub(in crate::rt) fn add_actor<'s>(&'s mut self) -> AddActor<&'s Mutex<Inactive>> {
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
    /// * `Err(())` if the scheduler currently can't be accessed.
    pub(in crate::rt) fn try_steal(&mut self) -> Result<Option<Pin<Box<ProcessData>>>, ()> {
        match self.shared.ready.try_read() {
            Some(run_queue) => Ok(run_queue.remove()),
            None => Err(()),
        }
    }

    /// Add back a process that was previously removed via
    /// [`WorkStealer::try_steal`].
    pub(in crate::rt) fn add_process(&mut self, process: Pin<Box<ProcessData>>) {
        self.shared.inactive.lock().add(process)
    }
}

impl fmt::Debug for SchedulerRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SchedulerRef")
    }
}

impl<'s> AddActor<&'s Mutex<Inactive>> {
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
        S: Supervisor<NA> + Send + 'static,
        NA: NewActor<Context = context::ThreadSafe> + Send + 'static,
        NA::Actor: Send + 'static,
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
