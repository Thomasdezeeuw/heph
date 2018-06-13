//! Module containing the scheduler.

use system::process::ProcessPtr;

mod priority;

pub use self::priority::Priority;

/// The scheduler, responsible for scheduling and running processes.
///
/// The scheduler implements the `Iterator` trait which can used iterate over
/// all scheduled processes.
pub struct Scheduler {
    queue: Vec<ProcessPtr>,
}

impl Scheduler {
    /// Create a new scheduler.
    pub fn new() -> Scheduler {
        Scheduler {
            queue: Vec::new(),
        }
    }

    /// Schedule a process.
    pub fn schedule(&mut self, process: ProcessPtr) {
        // TODO: take priority of the process into account.
        self.queue.push(process);
    }
}

impl<'a> Iterator for &'a mut Scheduler {
    type Item = ProcessPtr;

    fn next(&mut self) -> Option<Self::Item> {
        self.queue.pop()
    }
}
