//! Module with timers.

use std::cmp::Reverse;
use std::collections::binary_heap::{BinaryHeap, PeekMut};
use std::iter::FusedIterator;
use std::time::Instant;

use crate::rt::ProcessId;

/// Timer readiness queue.
///
/// Polling this event source never returns an error.
#[derive(Debug)]
pub(crate) struct Timers {
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
    pub(crate) fn new() -> Timers {
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

    /// Remove the next deadline that passed `now` returning the pid.
    pub(super) fn remove_deadline(&mut self, now: Instant) -> Option<ProcessId> {
        match self.deadlines.peek_mut() {
            Some(deadline) if deadline.0.deadline <= now => {
                let deadline = PeekMut::pop(deadline).0;
                Some(deadline.pid)
            }
            _ => None,
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
        self.timers.remove_deadline(self.now)
    }
}

impl<'t> FusedIterator for Deadlines<'t> {}
