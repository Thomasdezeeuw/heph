//! Module containing the scheduler `Priority` type.

use std::cmp::Ordering;

// TODO: Document the priorities more including the effects it has on
// scheduling, once that is implemented.

/// Priority for an actor in the scheduler.
//
// The priority ranges from `Priority::MIN` to `Priority::MAX`, where `MAX` is
// 0.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
#[repr(transparent)]
pub struct Priority(u8);

impl Priority {
    /// Low priority.
    ///
    /// All actors have a higher priority then this actor.
    pub const LOW: Priority = Priority(15);

    /// Normal priority.
    ///
    /// Most actors should run at this priority, hence its also the default
    /// priority.
    pub const NORMAL: Priority = Priority(10);

    /// High priority.
    ///
    /// Takes priority over other actors.
    pub const HIGH: Priority = Priority(5);

    /* TODO: uncomment this code once the scheduler actually takes priority into
     * account.
    /// Lowest priority possible priority.
    const MIN: u8 = 19;

    /// Highest priority possible priority.
    const MAX: u8 = 0;

    /// Increase the priority.
    ///
    /// Takes care of overflows.
    pub(crate) fn increase(&mut self) {
        if self.0 != Self::MAX {
            self.0 -= 1;
        }
    }

    /// Decrease the priority.
    ///
    /// Takes care of underflows.
    pub(crate) fn decrease(&mut self) {
        if self.0 != Self::MIN {
            self.0 += 1;
        }
    }
    */
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
