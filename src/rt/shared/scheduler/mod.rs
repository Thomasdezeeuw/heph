//! Module with the thread-safe scheduler.
//!
//! Scheduler for the actors started with [`RuntimeRef::try_spawn`].
//!
//! [`RuntimeRef::try_spawn`]: crate::rt::RuntimeRef::try_spawn

use std::mem::MaybeUninit;
use std::pin::Pin;
use std::sync::Mutex;

use inbox::Manager;
use log::trace;

use crate::rt::options::Priority;
use crate::rt::process::{self, ActorProcess, Process, ProcessId};
use crate::rt::ThreadSafe;
use crate::{NewActor, Supervisor};

mod inactive;
mod runqueue;
#[cfg(test)]
mod tests;

use inactive::Inactive;
use runqueue::RunQueue;

pub(crate) type ProcessData = process::ProcessData<dyn Process + Send + Sync>;

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
/// ## Adding actors (processes)
///
/// Adding new actors to the scheduler is a two step process. First, the
/// resources are allocated in [`Scheduler::add_actor`], which returns an
/// [`AddActor`] structure. This `AddActor` can be used to determine the
/// [`ProcessId`] (pid) of the actor and can be used in setting up the actor,
/// before the actor itself is initialised.
///
/// Second, after the actor is initialised, it can be added to the scheduler
/// using [`AddActor::add`]. This adds to the [`RunQueue`] or [`Inactive`] list
/// depending on whether its ready to run.
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
/// [`Scheduler::add_process`], adding it back to the [`Inactive`] list, or
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
    inactive: Mutex<Inactive>,
}

impl Scheduler {
    /// Create a new `Scheduler`.
    pub(crate) fn new() -> Scheduler {
        Scheduler {
            ready: RunQueue::empty(),
            inactive: Mutex::new(Inactive::empty()),
        }
    }

    /// Returns `true` if the scheduler has any processes (in any state),
    /// `false` otherwise.
    pub(crate) fn has_process(&self) -> bool {
        let has_inactive = { self.inactive.lock().unwrap().has_process() };
        has_inactive || self.has_ready_process()
    }

    /// Returns `true` if the scheduler has any processes that are ready to run,
    /// `false` otherwise.
    pub(crate) fn has_ready_process(&self) -> bool {
        self.ready.has_process()
    }

    /// Add a new actor to the scheduler.
    pub(crate) fn add_actor<'s>(&'s self) -> AddActor<'s> {
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
    pub(crate) fn mark_ready(&self, pid: ProcessId) {
        trace!("marking process as ready: pid={}", pid);
        if let Some(process) = { self.inactive.lock().unwrap().mark_ready(pid) } {
            // The process was in the `Inactive` list, so we move it to the run
            // queue.
            self.ready.add(process)
        }
        // NOTE: if the process in currently not in the `Inactive` list it will
        // be marked as ready-to-run and `Scheduler::add_process` will add it to
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
    pub(crate) fn add_process(&self, process: Pin<Box<ProcessData>>) {
        let pid = process.as_ref().id();

        trace!("adding back process: pid={}", pid);
        if let Some(process) = { self.inactive.lock().unwrap().add(process) } {
            // If the process was marked as ready-to-run we need to add to the
            // run queue instead.
            self.ready.add(process);
        }
    }

    /// Mark `process` as complete, removing it from the scheduler.
    #[allow(clippy::unused_self)] // See NOTE below.
    pub(crate) fn complete(&self, process: Pin<Box<ProcessData>>) {
        let pid = process.as_ref().id();
        trace!("removing process: pid={}", pid);
        drop(process);

        // NOTE: we could leave a ready-to-run marker in the `Inactive` list,
        // but its not really worth it (locking other workers out) to remove it.
        // Once the `Inactive` is wait/lock-free this decision should be
        // reconsidered.
    }
}

/// A handle to add a process to the scheduler.
///
/// This allows the `ProcessId` to be determined before the process is actually
/// added. This is used in registering with the system poller.
pub(crate) struct AddActor<'s> {
    scheduler: &'s Scheduler,
    /// Already allocated `ProcessData`, used to determine the `ProcessId`.
    alloc: Box<MaybeUninit<ProcessData>>,
}

impl<'s> AddActor<'s> {
    /// Get the would be `ProcessId` for the process.
    pub(crate) const fn pid(&self) -> ProcessId {
        #[allow(trivial_casts)]
        ProcessId(unsafe { &*self.alloc as *const _ as *const u8 as usize })
    }

    /// Add a new thread-safe actor to the scheduler.
    pub(crate) fn add<S, NA>(
        self,
        priority: Priority,
        supervisor: S,
        new_actor: NA,
        actor: NA::Actor,
        inbox: Manager<NA::Message>,
        is_ready: bool,
    ) where
        S: Supervisor<NA> + Send + Sync + 'static,
        NA: NewActor<RuntimeAccess = ThreadSafe> + Send + Sync + 'static,
        NA::Actor: Send + Sync + 'static,
        NA::Message: Send,
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
            scheduler.ready.add(process);
        } else {
            scheduler.add_process(process)
        }
    }
}
