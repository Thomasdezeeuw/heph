//! Timers implementation.
//!
//! This module hold the timer**s** implementation, that is the collection of
//! timers currently in the runtime. Also see the [`timer`] implementation,
//! which exposes types to the user.
//!
//! [`timer`]: crate::timer

use std::task;
use std::time::{Duration, Instant};

mod timing_wheel;

pub(crate) use timing_wheel::{SharedTimingWheel, TimingWheel};

/// Timers implementation.
#[allow(clippy::len_without_is_empty)]
pub trait Timers {
    /// Returns the current total number of timers.
    ///
    /// # Notes
    ///
    /// This is only used for debugging & logging purposes.
    fn len(&self) -> usize;

    /// Returns the next deadline, if any.
    fn next(&mut self) -> Option<Instant>;

    /// Same as [`next`], but returns a [`Duration`] instead. If the next
    /// deadline is already passed this returns a duration of zero.
    ///
    /// [`next`]: Timers::next
    fn next_timer(&mut self) -> Option<Duration> {
        self.next().map(|deadline| {
            Instant::now()
                .checked_duration_since(deadline)
                .unwrap_or(Duration::ZERO)
        })
    }

    /// Add a new deadline.
    fn add(&mut self, deadline: Instant, waker: task::Waker) -> TimerToken;

    /// Remove a previously added deadline.
    fn remove(&mut self, deadline: Instant, token: TimerToken);

    /// Expire all timers that have elapsed based on `now`. Returns the amount
    /// of expired timers (used for debugging & logging purposes).
    ///
    /// # Safety
    ///
    /// `now` may never go backwards between calls.
    fn expire_timers(&mut self, now: Instant) -> usize;
}

/// Token used to expire a timer.
///
/// See [`Timers`].
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
