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

#[cfg(test)]
impl Cfs {
    pub(crate) fn set_fair_runtime(&mut self, fair_runtime: Duration) {
        self.fair_runtime = fair_runtime;
    }

    pub(crate) fn fair_runtime(&mut self) -> Duration {
        self.fair_runtime
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[rustfmt::skip]
    fn cfs() {
        let mut process_system = Cfs::new(Priority::SYSTEM);
        let mut process_high = Cfs::new(Priority::HIGH);
        let mut process_normal = Cfs::new(Priority::NORMAL);
        let mut process_low = Cfs::new(Priority::LOW);

        // If the `fair_runtime` is equal we only compare based on the priority.
        assert_eq!(Cfs::order(&process_system, &process_system), Ordering::Equal);
        assert_eq!(Cfs::order(&process_system, &process_high), Ordering::Greater);
        assert_eq!(Cfs::order(&process_system, &process_normal), Ordering::Greater);
        assert_eq!(Cfs::order(&process_system, &process_low), Ordering::Greater);

        assert_eq!(Cfs::order(&process_high, &process_system), Ordering::Less);
        assert_eq!(Cfs::order(&process_high, &process_high), Ordering::Equal);
        assert_eq!(Cfs::order(&process_high, &process_normal), Ordering::Greater);
        assert_eq!(Cfs::order(&process_high, &process_low), Ordering::Greater);

        assert_eq!(Cfs::order(&process_normal, &process_system), Ordering::Less);
        assert_eq!(Cfs::order(&process_normal, &process_high), Ordering::Less);
        assert_eq!(Cfs::order(&process_normal, &process_normal), Ordering::Equal);
        assert_eq!(Cfs::order(&process_normal, &process_low), Ordering::Greater);

        assert_eq!(Cfs::order(&process_low, &process_system), Ordering::Less);
        assert_eq!(Cfs::order(&process_low, &process_high), Ordering::Less);
        assert_eq!(Cfs::order(&process_low, &process_normal), Ordering::Less);
        assert_eq!(Cfs::order(&process_low, &process_low), Ordering::Equal);

        let start = Instant::now();
        let end = start;
        process_system.update(start, end, Duration::from_millis(10));
        assert_eq!(process_system.fair_runtime, Duration::from_millis(0));
        process_high.update(start, end, Duration::from_millis(5));
        assert_eq!(process_high.fair_runtime, Duration::from_millis(25));
        process_normal.update(start, end, Duration::from_millis(2));
        assert_eq!(process_normal.fair_runtime, Duration::from_millis(20));
        process_low.update(start, end, Duration::from_millis(1));
        assert_eq!(process_low.fair_runtime, Duration::from_millis(15));

        // If the `fair_runtime` is not equal we order by that.
        assert_eq!(Cfs::order(&process_system, &process_system), Ordering::Equal);
        assert_eq!(Cfs::order(&process_system, &process_high), Ordering::Greater);
        assert_eq!(Cfs::order(&process_system, &process_normal), Ordering::Greater);
        assert_eq!(Cfs::order(&process_system, &process_low), Ordering::Greater);

        assert_eq!(Cfs::order(&process_high, &process_system), Ordering::Less);
        assert_eq!(Cfs::order(&process_high, &process_high), Ordering::Equal);
        assert_eq!(Cfs::order(&process_high, &process_normal), Ordering::Less);
        assert_eq!(Cfs::order(&process_high, &process_low), Ordering::Less);

        assert_eq!(Cfs::order(&process_normal, &process_system), Ordering::Less);
        assert_eq!(Cfs::order(&process_normal, &process_high), Ordering::Greater);
        assert_eq!(Cfs::order(&process_normal, &process_normal), Ordering::Equal);
        assert_eq!(Cfs::order(&process_normal, &process_low), Ordering::Less);

        assert_eq!(Cfs::order(&process_low, &process_system), Ordering::Less);
        assert_eq!(Cfs::order(&process_low, &process_high), Ordering::Greater);
        assert_eq!(Cfs::order(&process_low, &process_normal), Ordering::Greater);
        assert_eq!(Cfs::order(&process_low, &process_low), Ordering::Equal);
    }
}
