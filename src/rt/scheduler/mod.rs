//! Module containing the `Scheduler` and related types.

use std::cmp::Ordering;
use std::fmt;
use std::mem::MaybeUninit;
use std::ops::Deref;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use log::trace;
use parking_lot::RwLock;

use crate::inbox::Inbox;
use crate::rt::process::{ActorProcess, Process, ProcessId, ProcessResult};
use crate::{NewActor, RuntimeRef, Supervisor};

mod inactive;
mod priority;
mod runqueue;

use inactive::Inactive;
use runqueue::RunQueue;

#[cfg(test)]
mod tests;

// # How the `Scheduler` works.
//
// There are two components to the scheduler:
//
// * `RunQueue`: holds the processes that are ready to run.
// * `Inactive`: holds the inactive processes.
//
// The scheduler has unique access to the `Inactive`. However the `RunQueue` is
// shared with `WorkStealer`, which can be used to steal processes in the ready
// state from this scheduler. This is used by other workers threads to steal
// work for themselves in an effort to prevent an in-balance in the workload.
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

pub use priority::Priority;

/// The scheduler, responsible for scheduling processes.
pub(super) struct Scheduler {
    /// Processes that are ready to run.
    ///
    /// # Notes
    ///
    /// This is shared with `WorkStealer`, which can steal ready to run
    /// processes from this scheduler.
    ///
    /// The lock may only be accessed via two methods:
    /// * `try_write`: attempts to get unique access to the processes.
    /// * `try_read`: if `try_write` fails another thread has access to it via a
    ///   `WorkStealer`. However `WorkStealer` may only use the read lock,
    ///   meaning that this owner of the scheduler (this struct) can always get
    ///   access to the processes, with an optimisation if its the only one with
    ///   access.
    ready: Arc<RwLock<RunQueue>>,
    /// Inactive processes that are not ready to run.
    inactive: Inactive,
}

impl Scheduler {
    /// Create a new `Scheduler` and accompanying reference.
    pub(super) fn new() -> (Scheduler, WorkStealer) {
        let ready = Arc::new(RwLock::new(RunQueue::empty()));
        let scheduler = Scheduler {
            ready: ready.clone(),
            inactive: Inactive::empty(),
        };
        let stealer = WorkStealer { ready };
        (scheduler, stealer)
    }

    /// Returns `true` if the schedule has any processes (in any state), `false`
    /// otherwise.
    pub(super) fn has_process(&self) -> bool {
        self.inactive.has_process() || self.has_ready_process()
    }

    /// Returns `true` if the schedule has any processes that are ready to run,
    /// `false` otherwise.
    pub(super) fn has_ready_process(&self) -> bool {
        self.run_queue(|run_queue| run_queue.has_process())
    }

    /// Add an actor to the scheduler.
    pub(super) fn add_actor<'s>(&'s mut self) -> AddActor<'s> {
        AddActor {
            processes: &mut self.inactive,
            alloc: Box::new_uninit(),
        }
    }

    /// Mark the process, with `pid`, as ready to run.
    ///
    /// # Notes
    ///
    /// Calling this with an invalid or outdated `pid` will be silently ignored.
    pub(super) fn mark_ready(&mut self, pid: ProcessId) {
        trace!("marking process as ready: pid={}", pid);
        if let Some(process) = self.inactive.remove(pid) {
            match self.ready.try_write() {
                // Fast path.
                Some(mut run_queue) => run_queue.add_mut(process),
                // Slow path.
                None => self.run_queue(|run_queue| run_queue.add(process)),
            }
        }
    }

    /// Returns the next ready process.
    pub(super) fn next_process(&mut self) -> Option<Pin<Box<ProcessData>>> {
        // Get a process from the run queue.
        match self.ready.try_write() {
            // Fast path.
            Some(mut run_queue) => run_queue.remove_mut(),
            // Slow path.
            None => self.run_queue(|run_queue| run_queue.remove()),
        }
    }

    /// Add back a process that was previously removed via
    /// [`Scheduler::next_process`].
    pub(super) fn add_process(&mut self, process: Pin<Box<ProcessData>>) {
        self.inactive.add(process)
    }

    /// Get immutable access to the run queue.
    fn run_queue<F, O>(&self, f: F) -> O
    where
        F: FnOnce(&RunQueue) -> O,
    {
        match self.ready.try_read() {
            Some(run_queue) => f(run_queue.deref()),
            // This is impossible as `Scheduler` is the only object that can use
            // the `try_write` method, to which we have a unique reference, thus
            // `try_read` will never fail.
            None => unreachable!("Scheduler can't get access to RunQueue"),
        }
    }
}

impl fmt::Debug for Scheduler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Scheduler")
    }
}

/// Handle to a [`Scheduler`] to steal processes from the run queue.
#[derive(Clone)]
pub struct WorkStealer {
    /// Processes that are ready to run.
    ///
    /// # Notes
    ///
    /// This lock may only be accessed via the `try_read` method. **No other
    /// methods are allowed to be used**.
    ready: Arc<RwLock<RunQueue>>,
}

impl WorkStealer {
    /// Attempts to steal a process.
    ///
    /// If this returns `Err(())` it means the thread that owns the scheduler
    /// has unique access to it and that we can't get access to it. Otherwise it
    /// returns a process or nothing if the run queue is empty.
    #[allow(dead_code)] // TODO: use this.
    pub(super) fn try_steal(&mut self) -> Result<Option<Pin<Box<ProcessData>>>, ()> {
        match self.ready.try_read() {
            Some(run_queue) => Ok(run_queue.remove()),
            None => Err(()),
        }
    }
}

impl fmt::Debug for WorkStealer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("WorkStealer")
    }
}

/// Data related to a process.
///
/// # Notes
///
/// `PartialEq` and `Eq` are implemented based on the id of the process
/// (`ProcessId`).
///
/// `PartialOrd` and `Ord` however are implemented based on runtime and
/// priority.
pub(super) struct ProcessData {
    priority: Priority,
    /// Fair runtime of the process, which is `actual runtime * priority`.
    fair_runtime: Duration,
    process: Pin<Box<dyn Process>>,
}

impl ProcessData {
    /// Returns the process identifier, or pid for short.
    fn id(self: Pin<&Self>) -> ProcessId {
        // Since the pid only job is to be unique we just use the pointer to
        // this structure as pid. This way we don't have to store any additional
        // pid in the structure itself or in the scheduler.
        #[allow(trivial_casts)]
        let ptr = unsafe { Pin::into_inner_unchecked(self) as *const _ as *const u8 };
        ProcessId(ptr as usize)
    }

    /// Run the process.
    ///
    /// Returns the completion state of the process.
    pub(super) fn run(mut self: Pin<&mut Self>, runtime_ref: &mut RuntimeRef) -> ProcessResult {
        let pid = self.as_ref().id();
        trace!("running process: pid={}", pid);

        let start = Instant::now();
        let result = self.process.as_mut().run(runtime_ref, pid);
        let elapsed = start.elapsed();
        let fair_elapsed = elapsed * self.priority;
        self.fair_runtime += fair_elapsed;

        trace!(
            "finished running process: pid={}, elapsed_time={:?}, result={:?}",
            pid,
            elapsed,
            result
        );

        result
    }
}

impl Eq for ProcessData {}

impl PartialEq for ProcessData {
    fn eq(&self, other: &Self) -> bool {
        // FIXME: is this safe?
        Pin::new(self).id() == Pin::new(other).id()
    }
}

impl Ord for ProcessData {
    fn cmp(&self, other: &Self) -> Ordering {
        (other.fair_runtime)
            .cmp(&(self.fair_runtime))
            .then_with(|| self.priority.cmp(&other.priority))
    }
}

impl PartialOrd for ProcessData {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Debug for ProcessData {
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
pub(super) struct AddActor<'s> {
    processes: &'s mut Inactive,
    /// Already allocated `ProcessData`, used to determine the `ProcessId`.
    alloc: Box<MaybeUninit<ProcessData>>,
}

impl<'s> AddActor<'s> {
    /// Get the would be `ProcessId` for the process.
    pub(super) const fn pid(&self) -> ProcessId {
        #[allow(trivial_casts)]
        ProcessId(unsafe { &*self.alloc as *const _ as *const u8 as usize })
    }

    /// Add a new inactive actor to the scheduler.
    pub(super) fn add<S, NA>(
        self,
        priority: Priority,
        supervisor: S,
        new_actor: NA,
        actor: NA::Actor,
        inbox: Inbox<NA::Message>,
    ) where
        S: Supervisor<NA> + 'static,
        NA: NewActor + 'static,
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
        processes.add(process)
    }
}
