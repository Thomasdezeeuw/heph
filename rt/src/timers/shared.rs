//! Thread-safe version of `Timers`.

use std::cmp::min;
use std::sync::RwLock;
use std::task::Waker;
use std::time::{Duration, Instant};

use crate::timers::{
    add_timer, remove_if_before, remove_timer, TimeOffset, Timer, TimerLocation, TimerToken,
    DURATION_PER_SLOT, NS_OVERFLOW, NS_PER_SLOT, NS_PER_SLOT_BITS, NS_SLOT_MASK, OVERFLOW_DURATION,
    SLOTS, SLOT_BITS,
};

/// Shared timers.
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

impl Timers {
    /// Create a new collection of timers.
    pub(crate) fn new() -> Timers {
        #[allow(clippy::declare_interior_mutable_const)]
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

    #[cfg(test)]
    pub(crate) fn epoch(&self) -> (Instant, u8) {
        let epoch = self.epoch.read().unwrap();
        (epoch.time, epoch.index)
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
                        }
                        // Process the next slot.
                        break;
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
    #[allow(clippy::cast_possible_truncation)] // For `epoch.index`.
    fn maybe_update_epoch(&self, now: Instant) -> bool {
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
            epoch.index = (epoch.index + 1) % self.slots.len() as u8;
            epoch.time = new_epoch;
            new_epoch
        };

        // Next move all the overflow timers that now fit in the slots.
        let time = epoch_time + OVERFLOW_DURATION;
        while let Ok(timer) = { remove_if_before(&mut self.overflow.write().unwrap(), time) } {
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
