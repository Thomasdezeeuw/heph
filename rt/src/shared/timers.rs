//! Module with the shared timers implementation.
//!
//! Also see the local timers implementation.

use std::cmp::min;
use std::sync::RwLock;
use std::task::Waker;
use std::time::{Duration, Instant};

use log::{as_debug, trace};

use crate::timer::TimerToken;

#[cfg(test)]
#[path = "timers_tests.rs"]
mod timers_tests;

/// Bits needed for the number of slots.
const SLOT_BITS: usize = 6;
/// Number of slots in the [`Timers`] wheel, 64.
const SLOTS: usize = 1 << SLOT_BITS;
/// Bits needed for the nanoseconds per slot.
const NS_PER_SLOT_BITS: usize = 30;
/// Nanoseconds per slot, 1073741824 ns ~= 1 second.
const NS_PER_SLOT: TimeOffset = 1 << NS_PER_SLOT_BITS;
/// Duration per slot, [`NS_PER_SLOT`] as [`Duration`].
const DURATION_PER_SLOT: Duration = Duration::from_nanos(NS_PER_SLOT as u64);
/// Timers within `((1 << 6) * (1 << 30))` ~= 68 seconds since the epoch fit in
/// the wheel, others get added to the overflow.
const NS_OVERFLOW: u64 = SLOTS as u64 * NS_PER_SLOT as u64;
/// Duration per slot, [`NS_OVERFLOW`] as [`Duration`].
const OVERFLOW_DURATION: Duration = Duration::from_nanos(NS_OVERFLOW);
/// Mask to get the nanoseconds for a slot.
const NS_SLOT_MASK: u128 = (1 << NS_PER_SLOT_BITS) - 1;

/// Time offset since the epoch of [`Timers::epoch`].
///
/// Must fit [`NS_PER_SLOT`].
type TimeOffset = u32;

/// Timers.
///
/// This implementation is based on a Timing Wheel as discussed in the paper
/// "Hashed and hierarchical timing wheels: efficient data structures for
/// implementing a timer facility" by George Varghese and Anthony Lauck (1997).
///
/// This uses a scheme that splits the timers based on when they're going to
/// expire. It has 64 ([`SLOTS`]) slots each representing roughly a second of
/// time ([`NS_PER_SLOT`]). This allows us to only consider a portion of all
/// timers when processing the timers. Any timers that don't fit into these
/// slots, i.e. timers with a deadline more than 68 seconds ([`NS_OVERFLOW`])
/// past `epoch`, are put in a overflow list. Ideally this overflow list is
/// empty however.
///
/// The `slots` hold the timers with a [`TimeOffset`] which is the number of
/// nanosecond since epoch times it's index. The `index` field determines the
/// current zero-slot, meaning its timers will expire next and all have a
/// deadline within `0..NS_PER_SLOT` nanoseconds after `epoch`. The
/// `slots[index+1]` list will have timers that expire
/// `NS_PER_SLOT..2*NS_PER_SLOT` nanoseconds after `epoch`. In other words each
/// slot holds the timers that expire in the ~second after the previous slot.
///
/// Whenever timers are removed by `remove_next` it will attempt to update the
/// `epoch`, which is used as anchor point to determine in what slot/overflow
/// the timer must go (see above). When updating the epoch it will increase the
/// `index` by 1 and the `epoch` by [`NS_PER_SLOT`] nanoseconds in a single
/// atomic step (thus requiring a lock around `Epoch`). This means the next slot
/// (now `slots[index+1]`) holds timers that expire `0..NS_PER_SLOT` nanoseconds
/// after `epoch`.
///
/// Note that it's possible for a thread to read the epoch (index and time),
/// than gets descheduled, another thread updates the epoch and finally the
/// second thread insert a timer based on a now outdated epoch. This situation
/// is fine as the timer will still be added to the correct slot, but it has a
/// higher change of being added to the overflow list (which
/// `maybe_update_epoch` deals with correctly).
#[derive(Debug)]
pub(crate) struct Timers {
    epoch: RwLock<Epoch>,
    /// The vectors are sorted.
    slots: [RwLock<Vec<Timer<TimeOffset>>>; SLOTS],
    /// The vector is sorted.
    overflow: RwLock<Vec<Timer<Instant>>>,
}

/// Separate struct because both fields need to be updated atomically.
#[derive(Debug)]
struct Epoch {
    time: Instant,
    index: u8,
}

/// A timer in [`Timers`].
#[derive(Debug)]
struct Timer<T> {
    deadline: T,
    waker: Waker,
}

impl Timers {
    /// Create a new collection of timers.
    pub(crate) fn new() -> Timers {
        const EMPTY: RwLock<Vec<Timer<TimeOffset>>> = RwLock::new(Vec::new());
        Timers {
            epoch: RwLock::new(Epoch {
                time: Instant::now(),
                index: 0,
            }),
            slots: [EMPTY; SLOTS],
            overflow: RwLock::new(Vec::new()),
        }
    }

    /// Returns the total number of timers.
    pub(crate) fn len(&self) -> usize {
        let mut timers = 0;
        for slots in &self.slots {
            timers += slots.read().unwrap().len();
        }
        timers += self.overflow.read().unwrap().len();
        timers
    }

    /// Returns the next deadline, if any.
    pub(crate) fn next(&self) -> Option<Instant> {
        let (epoch_time, index) = {
            let epoch = self.epoch.read().unwrap();
            (epoch.time, epoch.index as usize)
        };
        let (second, first) = self.slots.split_at(index);
        let iter = first.iter().chain(second.iter());
        for (n, slot) in iter.enumerate() {
            if let Some(deadline) = { slot.read().unwrap().last().map(|timer| timer.deadline) } {
                let ns_since_epoch = u64::from(deadline) + (n as u64 * u64::from(NS_PER_SLOT));
                let deadline = epoch_time + Duration::from_nanos(ns_since_epoch);
                return Some(deadline);
            }
        }

        #[rustfmt::skip]
        self.overflow.read().unwrap().last().map(|timer| timer.deadline)
    }

    /// Same as [`next`], but returns a [`Duration`] instead. If the next
    /// deadline is already passed this returns a duration of zero.
    ///
    /// [`next`]: Timers::next
    pub(crate) fn next_timer(&self) -> Option<Duration> {
        self.next().map(|deadline| {
            Instant::now()
                .checked_duration_since(deadline)
                .unwrap_or(Duration::ZERO)
        })
    }

    /// Add a new deadline.
    pub(crate) fn add(&self, deadline: Instant, waker: Waker) -> TimerToken {
        // NOTE: it's possible that we call `add_timer` based on an outdated
        // epoch.
        self.get_timers(deadline, |timers| match timers {
            TimerLocation::InSlot((timers, deadline)) => add_timer(timers, deadline, waker),
            TimerLocation::Overflow((timers, deadline)) => add_timer(timers, deadline, waker),
        })
    }

    /// Remove a previously added deadline.
    pub(crate) fn remove(&self, deadline: Instant, token: TimerToken) {
        self.get_timers(deadline, |timers| match timers {
            TimerLocation::InSlot((timers, deadline)) => remove_timer(timers, deadline, token),
            TimerLocation::Overflow((timers, deadline)) => remove_timer(timers, deadline, token),
        });
    }

    /// Determines in what list of timers a timer with `deadline` would be/go
    /// into. Then calls the function `f` with either a slot or the overflow
    /// list.
    fn get_timers<F, T>(&self, deadline: Instant, f: F) -> T
    where
        F: FnOnce(TimerLocation<'_>) -> T,
    {
        let (epoch_time, epoch_index) = {
            let epoch = self.epoch.read().unwrap();
            (epoch.time, epoch.index)
        };
        let ns_since_epoch = deadline.saturating_duration_since(epoch_time).as_nanos();
        if ns_since_epoch < u128::from(NS_OVERFLOW) {
            #[allow(clippy::cast_possible_truncation)] // OK to truncate.
            let deadline = (ns_since_epoch & NS_SLOT_MASK) as TimeOffset;
            let index = ((ns_since_epoch >> NS_PER_SLOT_BITS) & ((1 << SLOT_BITS) - 1)) as usize;
            let index = (epoch_index as usize + index) % SLOTS;
            let mut timers = self.slots[index].write().unwrap();
            f(TimerLocation::InSlot((&mut *timers, deadline)))
        } else {
            // Too far into the future to fit in the slots.
            let mut overflow = self.overflow.write().unwrap();
            f(TimerLocation::Overflow((&mut *overflow, deadline)))
        }
    }

    /// Expire all timers that have elapsed based on `now`. Returns the amount
    /// of expired timers.
    ///
    /// # Safety
    ///
    /// `now` may never go backwards between calls.
    pub(crate) fn expire_timers(&self, now: Instant) -> usize {
        trace!(now = as_debug!(now); "expiring timers");
        let mut amount = 0;
        loop {
            // NOTE: Each loop iteration needs to calculate the `epoch_offset`
            // as the epoch changes each iteration.
            let (epoch_time, index) = {
                let epoch = self.epoch.read().unwrap();
                (epoch.time, epoch.index as usize)
            };
            // SAFETY: `now` can't go backwards, otherwise this will panic.
            let epoch_offset = now.duration_since(epoch_time).as_nanos();
            // NOTE: this truncates, which is fine as we need a max of
            // `NS_PER_SLOT` anyway.
            #[allow(clippy::cast_possible_truncation)]
            let epoch_offset = min(epoch_offset, u128::from(TimeOffset::MAX)) as TimeOffset;

            loop {
                // NOTE: don't inline this in the `match` statement, it will
                // cause the log the be held for the entire match statement,
                // which we don't want.
                let result =
                    { remove_if_before(&mut self.slots[index].write().unwrap(), epoch_offset) };
                match result {
                    // Wake up the future.
                    Ok(timer) => {
                        timer.waker.wake();
                        amount += 1;
                        // Try another timer in this slot.
                        continue;
                    }
                    Err(true) => {
                        // SAFETY: slot is empty, which makes calling
                        // `maybe_update_epoch` OK.
                        if !self.maybe_update_epoch(now) {
                            // Didn't update epoch, no more timers to process.
                            return amount;
                        } else {
                            // Process the next slot.
                            break;
                        }
                    }
                    // Slot has timers with a deadline past `now`, so no more
                    // timers to process.
                    Err(false) => return amount,
                }
            }
        }
    }

    /// Attempt to update the epoch based on the current time.
    ///
    /// # Panics
    ///
    /// This panics if the current slot is not empty.
    fn maybe_update_epoch(&self, now: Instant) -> bool {
        trace!(now = as_debug!(now); "maybe updating epoch");
        let epoch_time = {
            let mut epoch = self.epoch.write().unwrap();
            let new_epoch = epoch.time + DURATION_PER_SLOT;
            if new_epoch > now {
                // Can't move to the next slot yet.
                return false;
            }

            // Can't have old timers with a different absolute time.
            debug_assert!(self.slots[epoch.index as usize].read().unwrap().is_empty());

            // Move to the next slot and update the epoch.
            #[allow(clippy::cast_possible_truncation)]
            epoch.index = (epoch.index + 1) % self.slots.len() as u8;
            epoch.time = new_epoch;
            new_epoch
        };
        trace!(epoch_time = as_debug!(epoch_time); "new epoch time");

        // Next move all the overflow timers that now fit in the slots.
        let time = epoch_time + OVERFLOW_DURATION;
        while let Ok(timer) = { remove_if_before(&mut self.overflow.write().unwrap(), time) } {
            trace!(timer = as_debug!(timer); "moving overflow timer into wheel");
            // NOTE: we can't use the same optimisation as we do in the local
            // version where we know that all timers removed here go into the
            // `self.index-1` slot.
            // Because `add` has to work with outdated epoch information it
            // could be that it add a timers to the overflow list which could
            // have fit in one of the slots. So we have to deal with that
            // possbility here.
            _ = self.add(timer.deadline, timer.waker);
        }
        true
    }
}

/// Location of a timer.
enum TimerLocation<'a> {
    /// In of the wheel's slots.
    InSlot((&'a mut Vec<Timer<TimeOffset>>, TimeOffset)),
    /// In the overflow vector.
    Overflow((&'a mut Vec<Timer<Instant>>, Instant)),
}

/// Add a new timer to `timers`, ensuring it remains sorted.
fn add_timer<T: Ord>(timers: &mut Vec<Timer<T>>, deadline: T, waker: Waker) -> TimerToken {
    let idx = match timers.binary_search_by(|timer| timer.deadline.cmp(&deadline)) {
        Ok(idx) | Err(idx) => idx,
    };
    let token = TimerToken(waker.as_raw().data() as usize);
    timers.insert(idx, Timer { deadline, waker });
    token
}

/// Remove a previously added `deadline` from `timers`, ensuring it remains sorted.
fn remove_timer<T: Ord>(timers: &mut Vec<Timer<T>>, deadline: T, token: TimerToken) {
    if let Ok(idx) = timers.binary_search_by(|timer| timer.deadline.cmp(&deadline)) {
        if timers[idx].waker.as_raw().data() as usize == token.0 {
            _ = timers.remove(idx);
        }
    }
}

/// Remove the first timer if it's before `time`.
///
/// Returns `Ok(timer)` if there is a timer with a deadline before `time`.
/// Otherwise this returns `Err(true)` if `timers` is empty or `Err(false)` if
/// the are more timers in `timers`, but none with a deadline before `time`.
fn remove_if_before<T: Ord>(timers: &mut Vec<Timer<T>>, time: T) -> Result<Timer<T>, bool> {
    match timers.last() {
        Some(timer) if timer.deadline <= time => Ok(timers.pop().unwrap()),
        Some(_) => Err(false),
        None => Err(true),
    }
}
