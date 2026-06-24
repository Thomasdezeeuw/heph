//! Runtime Internals.
//!
//! This module contains the various parts that make up the Heph runtime.

// TODO: maybe rename this? Not a fan of the repeating name.

use std::task;
use std::time::{Duration, Instant};

/// Timers implementation.
pub trait Timers {
    /// Returns the next deadline, if any.
    fn next_deadline(&mut self) -> Option<Instant>;

    /// Same as [`next_deadline`], but returns it as a [`Duration`] instead. If
    /// the next deadline is already passed this returns a duration of zero.
    ///
    /// [`next_deadline`]: Timers::next_deadline
    fn until_next_deadline(&mut self) -> Option<Duration> {
        self.next_deadline().map(|deadline| {
            Instant::now()
                .checked_duration_since(deadline)
                .unwrap_or(Duration::ZERO)
        })
    }

    /// Expire all timers that have elapsed based on `now`. Returns the amount
    /// of expired timers.
    ///
    /// # Safety
    ///
    /// `now` may never go backwards between calls.
    fn expire_timers(&mut self, now: Instant) -> usize;

    /// Add a new deadline.
    ///
    /// The returned token can be used to cancel a timer, see [`remove`].
    ///
    /// [`remove`]: Timers::remove
    fn add(&mut self, deadline: Instant, waker: task::Waker) -> TimerToken;

    /// Remove a previously added deadline.
    ///
    /// The `token` should be one returned by [`add`] along with the same
    /// `deadline`.
    ///
    /// [`add`]: Timers::add
    fn remove(&mut self, deadline: Instant, token: TimerToken);

    /// Returns the current total number of timers.
    fn len(&self) -> usize;
}

/// Shared timers implementation.
///
/// An implementation of [`Timers`] that is shared between worker threads, used
/// by timers that are created by thread-safe processes.
///
/// The methods on this trait are the same as on [`Timers`], but take a
/// reference to self, rather than a mutable reference.
pub trait SharedTimers {
    /// Returns the next deadline, if any.
    fn next_deadline(&self) -> Option<Instant>;

    /// Same as [`next_deadline`], but returns it as a [`Duration`] instead. If
    /// the next deadline is already passed this returns a duration of zero.
    ///
    /// [`next_deadline`]: Timers::next_deadline
    fn until_next_deadline(&self) -> Option<Duration> {
        self.next_deadline().map(|deadline| {
            Instant::now()
                .checked_duration_since(deadline)
                .unwrap_or(Duration::ZERO)
        })
    }

    /// Expire all timers that have elapsed based on `now`. Returns the amount
    /// of expired timers.
    ///
    /// # Safety
    ///
    /// `now` may never go backwards between calls.
    fn expire_timers(&self, now: Instant) -> usize;

    /// Add a new deadline.
    ///
    /// The returned token can be used to cancel a timer, see [`remove`].
    ///
    /// [`remove`]: Timers::remove
    fn add(&self, deadline: Instant, waker: task::Waker) -> TimerToken;

    /// Remove a previously added deadline.
    ///
    /// The `token` should be one returned by [`add`] along with the same
    /// `deadline`.
    ///
    /// [`add`]: Timers::add
    fn remove(&self, deadline: Instant, token: TimerToken);

    /// Returns the current total number of timers.
    fn len(&self) -> usize;
}

/// Token used to expire a timer.
#[derive(Copy, Clone, Debug)]
pub struct TimerToken(usize);

impl TimerToken {
    /// Create a new timer token.
    pub const fn new(data: usize) -> TimerToken {
        TimerToken(data)
    }

    /// Returns the data passed to [`TimerToken::new`].
    pub fn data(self) -> usize {
        self.0
    }
}

pub use crate::timing_wheel::{SharedTimingWheel, TimingWheel};
