//! Module with user space readiness event queue.

use std::vec::Drain;

use crate::system::ProcessId;

/// User space readiness queue.
///
/// A simple, single threaded user space readiness queue.
#[derive(Debug)]
pub struct Queue {
    events: Vec<ProcessId>,
}

impl Queue {
    /// Create a new user space readiness event queue.
    pub fn new() -> Queue {
        Queue {
            events: Vec::new(),
        }
    }

    /// Returns true if the queue contains no events.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Add a new process that is ready.
    pub fn add(&mut self, pid: ProcessId) {
        self.events.push(pid);
    }

    /// Get all events, removing them form the queue.
    pub fn events(&mut self) -> Drain<'_, ProcessId> {
        self.events.drain(..)
    }
}
