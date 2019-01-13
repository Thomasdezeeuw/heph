//! Module containing the `Scheduler` and related types.

use std::cell::RefMut;
use std::collections::BinaryHeap;
use std::mem;
use std::pin::Pin;
use std::task::LocalWaker;

use log::{debug, trace};
use slab::Slab;

use crate::actor::{Actor, NewActor};
use crate::mailbox::MailBox;
use crate::supervisor::Supervisor;
use crate::system::ActorSystemRef;
use crate::util::Shared;

mod process;

#[cfg(all(test, feature = "test"))]
mod tests;

use self::process::{ActorProcess, Process, ProcessResult};

pub use self::process::Priority;
pub use self::process::ProcessId;

/// The scheduler, responsible for scheduling and running processes.
#[derive(Debug)]
pub struct Scheduler {
    /// Active processes.
    active: BinaryHeap<Pin<Box<dyn Process>>>,
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

    /// Check if any processes are present in the scheduler.
    ///
    /// Used by the actor system to determine whether to stop running.
    pub fn is_empty(&self) -> bool {
        self.processes.borrow().is_empty()
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
        match process.as_mut().run(system_ref) {
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
    /// This API allows the `ProcessId` to be used before the process is actually
    /// added to scheduler.
    pub fn add_process<'s>(&'s mut self) -> AddingProcess<'s> {
        AddingProcess {
            processes: self.processes.borrow_mut()
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
        ProcessId(self.processes.len())
    }

    /// Add a new inactive actor process to the scheduler.
    pub fn add_actor<S, NA>(self, priority: Priority, supervisor: S,
        new_actor: NA, actor: NA::Actor, mailbox: Shared<MailBox<NA::Message>>,
        waker: LocalWaker
    )
        where S: Supervisor<<NA::Actor as Actor>::Error, NA::Argument> + 'static,
              NA: NewActor + 'static,
    {
        let pid = self.pid();
        debug!("adding new actor process: pid={}", pid);
        let process = Box::pin(ActorProcess::new(pid, priority, supervisor,
            new_actor, actor, mailbox, waker));
        self.add_process(pid, process)
    }

    /// Add a new process to the scheduler.
    fn add_process(mut self, pid: ProcessId, process: Pin<Box<dyn Process>>) {
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
    Inactive(Pin<Box<dyn Process>>),
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
    fn mark_active(&mut self) -> Pin<Box<dyn Process>> {
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
    fn mark_inactive(&mut self, process: Pin<Box<dyn Process>>) {
        match mem::replace(self, ProcessState::Inactive(process)) {
            ProcessState::Active => {},
            ProcessState::Inactive(_) =>
                unreachable!("tried to mark an inactive process as inactive"),
        }
    }
}
