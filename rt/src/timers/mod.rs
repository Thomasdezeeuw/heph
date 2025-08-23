//! Timers implementation.
//!
//! This module holds the [`Timers`] implementations, that is the collection of
//! timers currently in the runtime. Also see the [`timer`] module, which
//! exposes timer related types to the user.
//!
//! The main implemention is the [`TimingWheel`].
//!
//! [`timer`]: crate::timer

use std::task;
use std::time::{Duration, Instant};

mod timing_wheel;

pub(crate) use timing_wheel::SharedTimingWheel;
pub use timing_wheel::TimingWheel;

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

/// Very basic implementation, not ready for production.
impl Timers for Vec<(Instant, task::Waker)> {
    fn len(&self) -> usize {
        self.len()
    }

    fn next(&mut self) -> Option<Instant> {
        self.first().map(|(i, _)| *i)
    }

    fn add(&mut self, deadline: Instant, waker: task::Waker) -> TimerToken {
        let idx = match self.binary_search_by(|(i, _)| i.cmp(&deadline)) {
            Ok(idx) | Err(idx) => idx,
        };
        let token = TimerToken::for_waker(&waker);
        self.insert(idx, (deadline, waker));
        token
    }

    fn remove(&mut self, deadline: Instant, token: TimerToken) {
        if let Ok(idx) = self.binary_search_by(|(i, _)| i.cmp(&deadline)) {
            if token.is_for_waker(&self[idx].1) {
                _ = self.remove(idx);
            }
        }
    }

    fn expire_timers(&mut self, now: Instant) -> usize {
        let idx = match self.binary_search_by(|(i, _)| i.cmp(&now)) {
            Ok(idx) | Err(idx) => idx,
        };

        self.drain(..idx)
            .map(|(_, w)| {
                w.wake();
                1
            })
            .sum()
    }
}
