//! Module containing the `Scheduler` and related types.

use std::cell::RefMut;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::mem;
use std::time::{Duration, Instant};

use log::{debug, trace};
use slab::Slab;

use crate::process::{Process, ProcessId, ProcessResult};
use crate::system::ActorSystemRef;
use crate::util::Shared;

mod priority;

#[cfg(all(test, feature = "test"))]
mod tests;

pub use self::priority::Priority;

// TODO: benchmark boxing `ProcessData` and then inlining `Process`. Thus
// erasing the generic type of `ProcessData<P>` into `Box<dyn
// ScheduledProcess>`.

/// The scheduler, responsible for scheduling and running processes.
#[derive(Debug)]
pub struct Scheduler {
    /// Active processes.
    active: BinaryHeap<Box<ProcessData>>,
    /// All* processes in the scheduler.
    ///
    /// *Actually active processes are in the active list above, but still have
    /// a state in this list.
    ///
    /// This is shared with `SchedulerRef`, which can add inactive processes.
    processes: Shared<Slab<ProcessState>>,
}

impl Scheduler {
    /// Create a new scheduler and accompanying reference.
    pub fn new() -> (Scheduler, SchedulerRef) {
        let shared = Shared::new(Slab::new());
        let scheduler = Scheduler {
            active: BinaryHeap::new(),
            processes: shared.clone(),
        };
        let scheduler_ref = SchedulerRef {
            processes: shared,
        };
        (scheduler, scheduler_ref)
    }

    /// Schedule a process.
    ///
    /// This marks a process as active and will add it to the list of processes
    /// to run.
    ///
    /// # Notes
    ///
    /// Calling this with an invalid or outdated `pid` will be silently ignored.
    pub fn schedule(&mut self, pid: ProcessId) {
        trace!("scheduling process: pid={}", pid);
        if let Some(ref mut process) = self.processes.borrow_mut().get_mut(pid.0) {
            if !process.is_active() {
                self.active.push(process.mark_active());
            }
        }
    }

    /// Check if a process is ready for running.
    pub fn process_ready(&self) -> bool {
        !self.active.is_empty()
    }

    /// Run the next active process.
    ///
    /// Returns `true` if a process was run, `false` otherwise.
    pub fn run_process(&mut self, system_ref: &mut ActorSystemRef) -> bool {
        let mut process = match self.active.pop() {
            Some(process) => process,
            None => {
                debug!("no active processes left to run");
                return false;
            },
        };

        let pid = process.id();
        match process.run(system_ref) {
            ProcessResult::Complete => {
                let process_state = self.processes.borrow_mut().remove(pid.0);
                debug_assert!(process_state.is_active(), "removed an inactive process");
            },
            ProcessResult::Pending => match self.processes.borrow_mut().get_mut(pid.0) {
                Some(ref mut process_state) =>
                    process_state.mark_inactive(process),
                None =>
                    unreachable!("active process already removed"),
            },
        }
        true
    }
}

/// A reference to the `Scheduler` that allows adding processes.
#[derive(Debug)]
pub struct SchedulerRef {
    /// Processes shared with `Scheduler`.
    processes: Shared<Slab<ProcessState>>,
}

impl SchedulerRef {
    /// Add a new process to the scheduler.
    ///
    /// By default the process will be considered inactive and thus not
    /// scheduled. To schedule the process see `Scheduler.schedule`.
    ///
    /// The API allows the `ProcessId` to be used before the process is actually
    /// added to scheduler.
    pub fn add_process(&mut self) -> AddingProcess {
        let processes = self.processes.borrow_mut();
        AddingProcess {
            id: ProcessId(processes.len()),
            processes,
        }
    }
}

/// A handle to add a process to the scheduler.
///
/// This allows the `ProcessId` to be determined before the process is actually
/// added. This is used in registering with the system poller.
pub struct AddingProcess<'s> {
    id: ProcessId,
    processes: RefMut<'s, Slab<ProcessState>>,
}

impl<'s> AddingProcess<'s> {
    /// Get the would be `ProcessId` for the process.
    pub fn id(&self) -> ProcessId {
        self.id
    }

    /// Add a new inactive process to the scheduler.
    pub fn add<P>(mut self, process: P, priority: Priority)
        where P: Process + 'static,
    {
        let pid = self.id;
        debug!("adding new process: pid={}", pid);
        let process = Box::new(ProcessData::new(pid, priority, process));
        let actual_pid = self.processes.insert(ProcessState::Inactive(process));
        debug_assert_eq!(actual_pid, pid.0);
    }
}

/// The state of a process.
#[derive(Debug)]
enum ProcessState {
    /// Process is currently active and can be found in the active list of
    /// processes.
    Active,
    /// Process is currently inactive.
    Inactive(Box<ProcessData>),
}

impl ProcessState {
    /// Whether or not the process is currently active.
    fn is_active(&self) -> bool {
        match self {
            ProcessState::Active => true,
            _ => false,
        }
    }

    /// Mark the process as active, returning the process data.
    ///
    /// # Panics
    ///
    /// Panics if the process was already active.
    fn mark_active(&mut self) -> Box<ProcessData> {
        match mem::replace(self, ProcessState::Active) {
            ProcessState::Active =>
                unreachable!("tried to mark an active process as active"),
            ProcessState::Inactive(process) => process,
        }
    }

    /// Mark the process as inactive.
    ///
    /// # Panics
    ///
    /// Panics if the process was already inactive.
    fn mark_inactive(&mut self, process: Box<ProcessData>) {
        match mem::replace(self, ProcessState::Inactive(process)) {
            ProcessState::Active => {},
            ProcessState::Inactive(_) =>
                unreachable!("tried to mark an inactive process as inactive"),
        }
    }
}

/// Structure that holds a process and it's data.
///
/// Equality is implemented based on the `ProcessId`. Ordering however is based
/// on the `fair_runtime` (Duration) and `Priority`, in that order.
#[derive(Debug)]
pub struct ProcessData {
    id: ProcessId,
    /// Runtime of this process * Priority, see the `update_runtime` method.
    fair_runtime: Duration,
    priority: Priority,
    process: Box<dyn Process>,
}

impl Eq for ProcessData {}

impl PartialEq for ProcessData {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Ord for ProcessData {
    fn cmp(&self, other: &Self) -> Ordering {
        other.fair_runtime.cmp(&self.fair_runtime)
            .then_with(|| self.priority.cmp(&other.priority))
    }
}

impl PartialOrd for ProcessData {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(&other))
    }
}

impl ProcessData {
    /// Create new `ProcessData`.
    pub fn new<P>(id: ProcessId, priority: Priority, process: P) -> ProcessData
        where P: Process + 'static,
    {
        ProcessData {
            id,
            fair_runtime: Duration::from_millis(0),
            priority,
            process: Box::new(process),
        }
    }

    /// The process id of this process.
    pub fn id(&self) -> ProcessId {
        self.id
    }

    /// Run the process and update it's fair runtime.
    pub fn run(&mut self, system_ref: &mut ActorSystemRef) -> ProcessResult {
        trace!("running process: pid={}", self.id);

        let start = Instant::now();
        let result = self.process.run(system_ref);
        let elapsed = start.elapsed();

        trace!("finished running process: pid={}, elapsed_time={:?}", self.id, elapsed);

        self.update_runtime(elapsed);
        result
    }

    /// Update the runtime of the process.
    fn update_runtime(&mut self, elapsed: Duration) {
        self.fair_runtime += elapsed * self.priority
    }
}
