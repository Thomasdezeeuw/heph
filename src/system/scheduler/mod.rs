//! Module containing the scheduler.

use std::fmt;
use std::collections::HashMap;

use system::process::{ProcessCompletion, ProcessId, ProcessIdGenerator, ProcessPtr};

mod priority;

pub use self::priority::Priority;

/// The scheduler, responsible for scheduling and running processes.
///
/// The scheduler implements the `Iterator` trait which can used iterate over
/// all scheduled processes.
pub struct Scheduler {
    /// Active, scheduled processes.
    queue: Vec<ProcessPtr>,
    /// Inactive processes.
    inactive: HashMap<ProcessId, ProcessPtr>,
    /// A generator for unique process ids.
    pid_gen: ProcessIdGenerator,
}

impl Scheduler {
    /// Create a new scheduler.
    pub fn new() -> Scheduler {
        Scheduler {
            queue: Vec::new(),
            inactive: HashMap::new(),
            pid_gen: ProcessIdGenerator::new(),
        }
    }

    /// Add a new process to the scheduler.
    ///
    /// By default the process will be considered inactive and thus not
    /// scheduled. To schedule the process see `schedule`.
    pub fn add_process(&mut self, mut process: ProcessPtr) -> ProcessId {
        let pid = self.pid_gen.next();
        process.set_id(pid);
        debug_assert_eq!(process.id(), pid, "process failed to set process id");
        self.add_inactive(process);
        pid
    }

    /// Add the process to the inactive processes list.
    fn add_inactive(&mut self, process: ProcessPtr) {
        if !self.inactive.insert(process.id(), process).is_none() {
            panic!("overwritten a process in inactive map");
        }
    }

    /// Schedule a process.
    ///
    /// This marks a process as active and moves it the scheduled queue.
    pub fn schedule(&mut self, pid: ProcessId) -> Result<(), ScheduleError> {
        let process = self.inactive.remove(&pid);
        if let Some(process) = process {
            debug_assert_eq!(process.id(), pid, "process has different pid then expected");
            // TODO: take priority of the process into account.
            self.queue.push(process);
            Ok(())
        } else {
            Err(ScheduleError)
        }
    }

    /// Run the scheduled processes.
    ///
    /// This loops over all currently scheduled processes and runs them.
    pub fn run(&mut self) {
        loop {
            match self.queue.pop() {
                Some(mut process) => match process.run() {
                    ProcessCompletion::Complete => drop(process),
                    ProcessCompletion::Pending => self.add_inactive(process),
                },
                None => return,
            }
        }
    }
}

impl fmt::Debug for Scheduler {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Scheduler")
            .field("queue (length)", &self.queue.len())
            .field("inactive (length)", &self.inactive.len())
            .finish()
    }
}

/// Error returned by [scheduling] a process.
///
/// This can mean one of two things:
/// 1. provided `ProcessId` is incorrect, or
/// 2. the process is already scheduled.
///
/// [scheduling]: struct.Scheduler.html#method.schedule
#[derive(Debug)]
pub struct ScheduleError;
