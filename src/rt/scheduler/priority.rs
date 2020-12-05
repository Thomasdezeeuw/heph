//! Module containing the scheduler `Priority` type.

use std::cmp::Ordering;
use std::num::NonZeroU8;
use std::ops::Mul;
use std::time::Duration;

// NOTE: the `Priority` type is public in the rt module. Because of this it
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
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct Priority(NonZeroU8);

impl Priority {
    /// Low priority.
    ///
    /// Other actors have priority over this actor.
    pub const LOW: Priority = Priority(NonZeroU8::new(15).unwrap());

    /// Normal priority.
    ///
    /// Most actors should run at this priority, hence its also the default
    /// priority.
    pub const NORMAL: Priority = Priority(NonZeroU8::new(10).unwrap());

    /// High priority.
    ///
    /// Takes priority over other actors.
    pub const HIGH: Priority = Priority(NonZeroU8::new(5).unwrap());
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

/// Implementation detail, please ignore.
#[doc(hidden)]
impl Mul<Priority> for Duration {
    type Output = Duration;

    fn mul(self, rhs: Priority) -> Duration {
        self * u32::from(rhs.0.get())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::Priority;

    #[test]
    #[allow(clippy::eq_op)] // Need to compare `Priority` to itself.
    fn priority() {
        assert!(Priority::HIGH > Priority::NORMAL);
        assert!(Priority::NORMAL > Priority::LOW);
        assert!(Priority::HIGH > Priority::LOW);

        assert_eq!(Priority::HIGH, Priority::HIGH);
        assert_ne!(Priority::HIGH, Priority::NORMAL);

        assert_eq!(Priority::default(), Priority::NORMAL);
    }

    #[test]
    fn priority_duration_multiplication() {
        let duration = Duration::from_millis(1);
        let high = duration * Priority::HIGH;
        let normal = duration * Priority::NORMAL;
        let low = duration * Priority::LOW;

        assert!(high < normal);
        assert!(normal < low);
        assert!(high < low);
    }
}
