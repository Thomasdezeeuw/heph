//! Basic implementation of a Completely Fair Scheduler (cfs) inspired scheduler
//! implementation.
//!
//! See <https://en.wikipedia.org/wiki/Completely_Fair_Scheduler> for more
//! information about CFS.

use std::cmp::Ordering;
use std::time::{Duration, Instant};

use crate::scheduler::{Priority, ProcessData, Schedule};

pub(crate) enum Cfs {}

impl Schedule for Cfs {
    type ProcessData = Data;

    fn order(lhs: &Self::ProcessData, rhs: &Self::ProcessData) -> Ordering {
        (rhs.fair_runtime)
            .cmp(&(lhs.fair_runtime))
            .then_with(|| lhs.priority.cmp(&rhs.priority))
    }
}

pub(crate) struct Data {
    priority: Priority,
    /// Fair runtime of the process, which is `actual runtime * priority`.
    fair_runtime: Duration,
}

impl ProcessData for Data {
    fn new(priority: Priority) -> Self {
        Data {
            priority,
            fair_runtime: Duration::ZERO,
        }
    }

    fn update(&mut self, _: Instant, _: Instant, elapsed: Duration) {
        let fair_elapsed = elapsed * self.priority;
        self.fair_runtime += fair_elapsed;
    }
}
