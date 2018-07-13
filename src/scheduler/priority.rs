//! Module containing the scheduler `Priority` type.

use std::cmp::Ordering;
use std::num::NonZeroU8;

/// Priority for an actor in the scheduler.
///
/// Actors with a higher priority will be scheduled to run more often and
/// quicker (after they return `Poll::Pending`) then actors with a lower
/// priority.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
#[repr(transparent)]
pub struct Priority(NonZeroU8);

impl Priority {
    /// Low priority.
    ///
    /// All actors have a higher priority then this actor.
    pub const LOW: Priority = Priority(unsafe { NonZeroU8::new_unchecked(15) });

    /// Normal priority.
    ///
    /// Most actors should run at this priority, hence its also the default
    /// priority.
    pub const NORMAL: Priority = Priority(unsafe { NonZeroU8::new_unchecked(10) });

    /// High priority.
    ///
    /// Takes priority over other actors.
    pub const HIGH: Priority = Priority(unsafe { NonZeroU8::new_unchecked(5) });
}

impl Default for Priority {
    fn default() -> Priority {
        Priority::NORMAL
    }
}

impl Ord for Priority {
    fn cmp(&self, other: &Self) -> Ordering {
        other.0.cmp(&self.0)
    }
}

impl PartialOrd for Priority {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        other.0.partial_cmp(&self.0)
    }

    fn lt(&self, other: &Self) -> bool {
        other.0 < self.0
    }

    fn le(&self, other: &Self) -> bool {
        other.0 <= self.0
    }

    fn gt(&self, other: &Self) -> bool {
        other.0 > self.0
    }

    fn ge(&self, other: &Self) -> bool {
        other.0 >= self.0
    }
}
