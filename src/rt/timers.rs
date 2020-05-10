//! Module with timers.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::iter::FusedIterator;
use std::time::Instant;

use crate::rt::ProcessId;

/// Timer readiness queue.
///
/// Polling this event source never returns an error.
#[derive(Debug)]
pub(super) struct Timers {
    deadlines: BinaryHeap<Reverse<Deadline>>,
}

/// A deadline.
///
/// This must be ordered by `deadline`, then `id`.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct Deadline {
    deadline: Instant,
    pid: ProcessId,
}

impl Timers {
    /// Create a new time event source.
    pub(super) fn new() -> Timers {
        Timers {
            deadlines: BinaryHeap::new(),
        }
    }

    /// Returns the next deadline, if any.
    pub(super) fn next_deadline(&self) -> Option<Instant> {
        self.deadlines.peek().map(|deadline| deadline.0.deadline)
    }

    /// Add a new deadline.
    pub(super) fn add_deadline(&mut self, pid: ProcessId, deadline: Instant) {
        self.deadlines.push(Reverse(Deadline { pid, deadline }));
    }

    /// Returns all deadlines that have expired (i.e. deadline < now).
    pub(super) fn deadlines(&mut self) -> Deadlines<'_> {
        Deadlines {
            timers: self,
            now: Instant::now(),
        }
    }
}

#[derive(Debug)]
pub(super) struct Deadlines<'t> {
    timers: &'t mut Timers,
    now: Instant,
}

impl<'t> Iterator for Deadlines<'t> {
    type Item = ProcessId;

    fn next(&mut self) -> Option<Self::Item> {
        match self.timers.deadlines.peek() {
            Some(deadline) if deadline.0.deadline <= self.now => {
                let deadline = self.timers.deadlines.pop().unwrap().0;
                Some(deadline.pid)
            }
            _ => None,
        }
    }
}

impl<'t> FusedIterator for Deadlines<'t> {}
