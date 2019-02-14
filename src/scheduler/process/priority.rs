//! Module containing the `Priority` type.

use std::ops::Mul;
use std::time::Duration;

// NOTE: the `Priority` type is public in the system module. Because of this it
// talks only about actors, rather the processes, because in the public
// documentation process is never mentioned. Effectively actor can be replaced
// with process in the documentation below.

/// Priority for an actor in the scheduler.
///
/// Actors with a higher priority will be scheduled to run more often and
/// quicker (after they return [`Poll::Pending`]) then actors with a lower
/// priority.
///
/// [`Poll::Pending`]: std::task::Poll::Pending
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[non_exhaustive]
pub enum Priority {
    /// Low priority.
    ///
    /// Other actors have priority over this actor.
    Low,
    /// Normal priority.
    ///
    /// Most actors should run at this priority, hence its also the default
    /// priority.
    Normal,
    /// High priority.
    ///
    /// Takes priority over other actors.
    High,
}

impl Default for Priority {
    fn default() -> Priority {
        Priority::Normal
    }
}

/// Implementation detail, please ignore.
#[doc(hidden)]
impl Mul<Priority> for Duration {
    type Output = Duration;

    fn mul(self, rhs: Priority) -> Duration {
        self * match rhs {
            Priority::Low => 15,
            Priority::Normal => 10,
            Priority::High => 5,
        }
    }
}
