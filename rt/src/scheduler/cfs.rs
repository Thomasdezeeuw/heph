//! Basic implementation of a Completely Fair Scheduler (cfs) inspired scheduler
//! implementation.
//!
//! See <https://en.wikipedia.org/wiki/Completely_Fair_Scheduler> for more
//! information about CFS.

use std::cmp::Ordering;
use std::time::{Duration, Instant};

use crate::scheduler::{Priority, Schedule};

#[derive(Debug)]
pub(crate) struct Cfs {
    priority: Priority,
    /// Fair runtime of the process, which is `actual runtime * priority`.
    fair_runtime: Duration,
}

impl Schedule for Cfs {
    fn new(priority: Priority) -> Cfs {
        Cfs {
            priority,
            fair_runtime: Duration::ZERO,
        }
    }

    fn update(&mut self, _: Instant, _: Instant, elapsed: Duration) {
        let fair_elapsed = elapsed * self.priority;
        self.fair_runtime += fair_elapsed;
    }

    fn order(lhs: &Self, rhs: &Self) -> Ordering {
        (rhs.fair_runtime)
            .cmp(&(lhs.fair_runtime))
            .then_with(|| lhs.priority.cmp(&rhs.priority))
    }
}
