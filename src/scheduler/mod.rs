//! Module containing the `Scheduler` and related types.

use std::fmt;
use std::collections::HashSet;
use std::time::Instant;

use slab::{Slab, VacantEntry};

use process::{Process, ProcessCompletion, ProcessId};
use system::ActorSystemRef;

mod priority;

pub use self::priority::Priority;

/// The scheduler, responsible for scheduling and running processes.
#[derive(Debug)]
pub struct Scheduler {
    /// Which processes are scheduled to run, by process id.
    ///
    /// It could be that this contains ids for processes that are no longer in
    /// the scheduler, we just ignore those.
    // TODO: use custom, simple hasher that just return the underlying usize as
    // u64.
    scheduled: HashSet<ProcessId>,
    /// All processes in the scheduler.
    processes: Slab<Box<dyn ScheduledProcess>>,
}

impl Scheduler {
    /// Create a new scheduler.
    pub fn new() -> Scheduler {
        Scheduler {
            scheduled: HashSet::new(),
            processes: Slab::new(),
        }
    }

    /// Add a new process to the scheduler.
    ///
    /// By default the process will be considered inactive and thus not
    /// scheduled. To schedule the process see `schedule`.
    ///
    /// The API allows the `ProcessId` to be used before the process is actually
    /// added to scheduler.
    pub fn add_process<'s>(&'s mut self) -> AddingProcess<'s> {
        AddingProcess {
            entry: self.processes.vacant_entry(),
        }
    }

    /// Schedule a process.
    ///
    /// This marks a process as active and will schedule it to run on the next
    /// call to `run`.
    ///
    /// # Notes
    ///
    /// Calling this with an invalid or outdated pid will be silently ignored.
    pub fn schedule(&mut self, pid: ProcessId) {
        debug!("scheduling process: pid={}", pid);
        // Don't care if it's already scheduled or not.
        let _ = self.scheduled.insert(pid);
    }

    /// Returns the number of processes currently scheduled.
    pub fn scheduled(&self) -> usize {
        self.scheduled.len()
    }

    /// Run all scheduled processes.
    pub fn run(&mut self, system_ref: &mut ActorSystemRef) {
        debug!("running all scheduled processes");
        for pid in self.scheduled.drain() {
            let completion = {
                let process = match self.processes.get_mut(pid.0) {
                    Some(process) => process,
                    None => {
                        debug!("process scheduled, but no longer alive: pid={}", pid);
                        continue
                    },
                };

                let start = Instant::now();
                trace!("running process: pid={}", pid);
                let res = process.run(system_ref);
                trace!("finished running process: pid={}, elapsed_time={:?}", pid, start.elapsed());
                res
            };

            if let ProcessCompletion::Complete = completion {
                trace!("process completed, removing it: pid={}", pid);
                drop(self.processes.remove(pid.0))
            }
        }
    }
}

/// A handle to add a process to the scheduler.
///
/// This allows the `ProcessId`, or pid, to be determined before the process is
/// actually added. This is used in registering with the system poller.
pub struct AddingProcess<'s> {
    /// A reference to the entry in which we'll add the process.
    entry: VacantEntry<'s, Box<dyn ScheduledProcess>>,
}

impl<'s> AddingProcess<'s> {
    /// Get the would be `ProcessId` for the process to be added.
    pub fn id(&self) -> ProcessId {
        ProcessId(self.entry.key())
    }

    /// Add a new process to the scheduler.
    pub fn add<P>(self, process: P, priority: Priority)
        where P: Process + 'static,
    {
        let pid = self.id();
        debug!("adding new process: pid={}", pid);
        let process = Box::new(ProcessData { priority, process });
        let _ = self.entry.insert(process);
    }
}

/// A process that is scheduled in the `Scheduler`.
///
/// The only implementation is `ProcessData`, but using an trait object allows
/// us to erase the actual type of the process.
trait ScheduledProcess: fmt::Debug {
    /// Get the priority of the process.
    fn priority(&self) -> Priority;

    /// Run the process.
    fn run(&mut self, system_ref: &mut ActorSystemRef) -> ProcessCompletion;
}

/// Container for a `Process` that holds the id and priority and implements
/// `ScheduledProcess`.
#[derive(Debug)]
struct ProcessData<P> {
    priority: Priority,
    process: P,
}

impl<P> ScheduledProcess for ProcessData<P>
    where P: Process,
{
    fn priority(&self) -> Priority {
        self.priority
    }

    fn run(&mut self, system_ref: &mut ActorSystemRef) -> ProcessCompletion {
        self.process.run(system_ref)
    }
}
