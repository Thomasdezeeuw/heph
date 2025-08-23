//! Timers implementation.
//!
//! This module hold the timer**s** implementation, that is the collection of
//! timers currently in the runtime. Also see the [`timer`] implementation,
//! which exposes types to the user.
//!
//! [`timer`]: crate::timer

use std::task;

mod timing_wheel;

pub(crate) use timing_wheel::{SharedTimers, TimingWheel};

/// Token used to expire a timer.
#[derive(Copy, Clone, Debug)]
pub struct TimerToken(usize);

impl TimerToken {
    /// Create a token for `waker`.
    pub fn for_waker(waker: &task::Waker) -> TimerToken {
        TimerToken(waker.data().addr())
    }

    /// Returns true if this token was created for `waker`.
    pub fn is_for_waker(&self, waker: &task::Waker) -> bool {
        waker.data().addr() == self.0
    }
}
