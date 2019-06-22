//! Module containing the `Scheduler` and related types.

use std::cell::RefCell;
use std::cell::RefMut;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::pin::Pin;
use std::rc::Rc;
use std::time::{Duration, Instant};
use std::{fmt, mem};

use gaea::{event, Event};
use log::{debug, trace};
use slab::Slab;

use crate::actor::{Actor, NewActor};
use crate::inbox::Inbox;
use crate::supervisor::Supervisor;
use crate::system::process::{ActorProcess, Process, ProcessId, ProcessResult};
use crate::system::ActorSystemRef;

mod priority;

#[cfg(test)]
mod tests;

pub use priority::Priority;

/// The scheduler, responsible for scheduling and running processes.
#[derive(Debug)]
pub struct Scheduler {
    /// Active processes.
    active: BinaryHeap<ProcessData>,
    /// All* processes in the scheduler.
    ///
    /// This is shared with `SchedulerRef`, which can add inactive processes.
    ///
    /// *Actually active processes are in the active list above, but still have
    /// a state in this list.
    processes: Rc<RefCell<Slab<ProcessState>>>,
}

impl Scheduler {
    /// Create a new scheduler and accompanying reference.
    pub fn new() -> (Scheduler, SchedulerRef) {
        let shared = Rc::new(RefCell::new(Slab::new()));
        let scheduler = Scheduler {
            active: BinaryHeap::new(),
            processes: shared.clone(),
        };
        let scheduler_ref = SchedulerRef { processes: shared };
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
        let pid = pid.0 as usize;
        if let Some(ref mut process) = self.processes.borrow_mut().get_mut(pid) {
            if !process.is_active() {
                self.active.push(process.mark_active());
            }
        }
    }

    /// Check if any processes are present in the scheduler.
    ///
    /// Used by the actor system to determine whether to stop running.
    pub fn is_empty(&self) -> bool {
        self.processes.borrow().is_empty()
    }

    /// Returns true is there are any processes ready to run, i.e. active.
    pub fn has_active_process(&self) -> bool {
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
            }
        };

        let pid = process.id.0 as usize;
        match process.run(system_ref) {
            ProcessResult::Complete => {
                let process_state = self.processes.borrow_mut().remove(pid);
                debug_assert!(process_state.is_active(), "removed an inactive process");
            }
            ProcessResult::Pending => match self.processes.borrow_mut().get_mut(pid) {
                Some(ref mut process_state) => process_state.mark_inactive(process),
                None => unreachable!("active process already removed"),
            },
        }
        true
    }
}

impl event::Sink for Scheduler {
    fn capacity_left(&self) -> event::Capacity {
        event::Capacity::Growable
    }

    fn add(&mut self, event: Event) {
        self.schedule(event.id().into());
    }
}

/// A reference to the `Scheduler` that allows adding processes.
#[derive(Debug)]
pub struct SchedulerRef {
    /// Processes shared with `Scheduler`.
    processes: Rc<RefCell<Slab<ProcessState>>>,
}

impl SchedulerRef {
    /// Add a new process to the scheduler.
    ///
    /// By default the process will be considered inactive and thus not
    /// scheduled. To schedule the process see `Scheduler.schedule`.
    ///
    /// This API allows the `ProcessId` to be used before the process is
    /// actually added to scheduler.
    pub fn add_process<'a>(&'a mut self) -> AddingProcess<'a> {
        AddingProcess {
            processes: self.processes.borrow_mut(),
        }
    }
}

/// A handle to add a process to the scheduler.
///
/// This allows the `ProcessId` to be determined before the process is actually
/// added. This is used in registering with the system poller.
pub struct AddingProcess<'s> {
    processes: RefMut<'s, Slab<ProcessState>>,
}

impl<'s> AddingProcess<'s> {
    /// Get the would be `ProcessId` for the process.
    pub fn pid(&self) -> ProcessId {
        ProcessId(self.processes.len() as u32)
    }

    /// Add a new inactive actor process to the scheduler.
    pub fn add_actor<S, NA>(
        self,
        priority: Priority,
        supervisor: S,
        new_actor: NA,
        actor: NA::Actor,
        mailbox: Inbox<NA::Message>,
    ) where
        S: Supervisor<<NA::Actor as Actor>::Error, NA::Argument> + 'static,
        NA: NewActor + 'static,
    {
        let process = Box::pin(ActorProcess::new(supervisor, new_actor, actor, mailbox));
        self.add_process(process, priority)
    }

    /// Add a new process to the scheduler.
    fn add_process(mut self, process: Pin<Box<dyn Process>>, priority: Priority) {
        let id = self.pid();
        debug!("adding new actor process: pid={}", id);
        let process_data = ProcessData {
            id,
            priority,
            runtime: Duration::from_millis(0),
            process,
        };
        let actual_pid = self.processes.insert(ProcessState::Inactive(process_data));
        debug_assert_eq!(actual_pid as u32, id.0);
    }
}

/// The state of a process.
#[derive(Debug)]
enum ProcessState {
    /// Process is currently active and can be found in the active list of
    /// processes.
    Active,
    /// Process is currently inactive.
    Inactive(ProcessData),
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
    fn mark_active(&mut self) -> ProcessData {
        match mem::replace(self, ProcessState::Active) {
            ProcessState::Active => unreachable!("tried to mark an active process as active"),
            ProcessState::Inactive(process) => process,
        }
    }

    /// Mark the process as inactive.
    ///
    /// # Panics
    ///
    /// Panics if the process was already inactive.
    fn mark_inactive(&mut self, process: ProcessData) {
        match mem::replace(self, ProcessState::Inactive(process)) {
            ProcessState::Active => {}
            ProcessState::Inactive(_) => {
                unreachable!("tried to mark an inactive process as inactive")
            }
        }
    }
}

/// Data related to a process.
///
/// # Notes
///
/// Because this structure moves around a lot inside the `Scheduler`, from
/// inactive to active and vice versa, it important for the performance for this
/// to be small in size.
struct ProcessData {
    id: ProcessId,
    priority: Priority,
    runtime: Duration,
    process: Pin<Box<dyn Process + 'static>>,
}

impl Eq for ProcessData {}

impl PartialEq for ProcessData {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Ord for ProcessData {
    fn cmp(&self, other: &Self) -> Ordering {
        (other.runtime * other.priority)
            .cmp(&(self.runtime * self.priority))
            .then_with(|| self.priority.cmp(&other.priority))
    }
}

impl PartialOrd for ProcessData {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl ProcessData {
    fn run(&mut self, system_ref: &mut ActorSystemRef) -> ProcessResult {
        trace!("running process: pid={}", self.id);

        let start = Instant::now();
        let result = self.process.as_mut().run(system_ref, self.id);
        let elapsed = start.elapsed();
        self.runtime += elapsed;

        trace!(
            "finished running process: pid={}, elapsed_time={:?}, result={:?}",
            self.id,
            elapsed,
            result
        );

        result
    }
}

impl fmt::Debug for ProcessData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Process")
            .field("id", &self.id)
            .field("priority", &self.priority)
            .field("runtime", &self.runtime)
            .finish()
    }
}
