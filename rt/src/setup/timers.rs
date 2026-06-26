//! Timers implementation.
//!
//! Trait defines the timers implementations that back the timers found in the
//! [`timer`] module.
//!
//! The [`Timers`] trait defines the implementation for timers used by
//! thread-local processes (actors and futures). Defaulting to [`TimingWheel`].
//!
//! Similarly the [`SharedTimers`] trait defines the implementation for
//! thread-safe processes. Defaulting to [`SharedTimingWheel`].
//!
//! [`timer`]: crate::timer

use std::time::{Duration, Instant};
use std::{fmt, task};

pub use crate::timing_wheel::{SharedTimingWheel, TimingWheel};

/// Timers implementation.
///
/// This implementation is used only for the thread-local timers implementation.
/// See [`SharedTimers`] for the thread-safe implementation, which is mostly the
/// same but uses a reference instead of a mutable reference.
pub trait Timers: fmt::Debug {
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
    /// `now` *must* not go backwards between calls.
    fn expire_timers(&mut self, now: Instant) -> usize;

    /// Add a new deadline.
    ///
    /// The returned token can be used to cancel a timer, see [`remove`].
    ///
    /// [`remove`]: Timers::remove
    ///
    /// # Notes
    ///
    /// The returned token *may* go unused if the timer expires and is never
    /// removed, no resources should be leaked in this case.
    fn add(&mut self, deadline: Instant, waker: task::Waker) -> TimerToken;

    /// Remove a previously added deadline.
    ///
    /// The `token` should be one returned by [`add`] along with the same
    /// `deadline`.
    ///
    /// [`add`]: Timers::add
    ///
    /// # Notes
    ///
    /// Only when the `deadline` is the same as passed to [`add`] and the
    /// returned `token` is this guaranteed to work, other usage is considered
    /// invalid.
    fn remove(&mut self, deadline: Instant, token: TimerToken);

    /// Returns the current total number of timers.
    fn len(&self) -> usize;
}

/// Uses [`TimingWheel`] as default timers implementation.
pub(crate) type DefaultTimers = fn() -> TimingWheel;

/// Shared timers implementation.
///
/// An implementation of [`Timers`] that is shared between worker threads, used
/// by timers that are created by thread-safe processes.
///
/// The methods on this trait are the same as on [`Timers`], but take a
/// reference to self, rather than a mutable reference.
pub trait SharedTimers: fmt::Debug {
    /// Returns the next deadline, if any.
    ///
    /// See [`Timers::next_deadline`].
    fn next_deadline(&self) -> Option<Instant>;

    /// Same as [`next_deadline`], but returns it as a [`Duration`] instead. If
    /// the next deadline is already passed this returns a duration of zero.
    ///
    /// See [`Timers::until_next_deadline`].
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
    /// See [`Timers::expire_timers`].
    fn expire_timers(&self, now: Instant) -> usize;

    /// Add a new deadline.
    ///
    /// See [`Timers::add`].
    fn add(&self, deadline: Instant, waker: task::Waker) -> TimerToken;

    /// Remove a previously added deadline.
    ///
    /// See [`Timers::remove`].
    fn remove(&self, deadline: Instant, token: TimerToken);

    /// Returns the current total number of timers.
    ///
    /// See [`Timers::len`].
    fn len(&self) -> usize;
}

/// Token used to expire a timer.
#[derive(Debug)]
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
