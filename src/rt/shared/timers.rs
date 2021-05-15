//! Module with the shared timers implementation.
//!
//! Also see the local timers implementation.

use std::cmp::{min, Ordering};
use std::sync::RwLock;
use std::time::{Duration, Instant};

use crate::rt::ProcessId;

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
/// ``Hashed and hierarchical timing wheels: efficient data structures for
/// implementing a timer facility'' by George Varghese and Anthony Lauck (1997).
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
/// nanosecond since epoch times it's index. The `index` filed determines the
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
/// second thread insert the time based on a now outdated epoch. This situation
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

/// Metrics for [`Timers`].
#[derive(Debug)]
pub(crate) struct Metrics {
    timers: usize,
    next_timer: Option<Duration>,
}

impl Timers {
    /// Create a new collection of timers.
    pub(crate) fn new() -> Timers {
        Timers {
            epoch: RwLock::new(Epoch {
                time: Instant::now(),
                index: 0,
            }),
            // TODO: replace with `RwLock::new(Vec::new()); SLOTS]` once
            // possible.
            #[rustfmt::skip]
            slots: [RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new()), RwLock::new(Vec::new())],
            overflow: RwLock::new(Vec::new()),
        }
    }

    /// Gather metrics about the timers.
    pub(crate) fn metrics(&self) -> Metrics {
        let next_timer = self.next().map(|deadline| {
            Instant::now()
                .checked_duration_since(deadline)
                .unwrap_or(Duration::ZERO)
        });
        let mut timers = 0;
        for slots in &self.slots {
            let slots = slots.read().unwrap();
            let len = slots.len();
            drop(slots);
            timers += len;
        }
        {
            let overflow = self.overflow.read().unwrap();
            timers += overflow.len()
        }
        Metrics { timers, next_timer }
    }

    /// Returns the next deadline, if any.
    ///
    /// If this return `Some` `woke_from_polling` must be called after polling,
    /// before removing timers. That thread must also wake other workers threads
    /// as they will see `None` here, **even if there is a timer set**.
    pub(crate) fn next(&self) -> Option<Instant> {
        let (epoch_time, index) = {
            let epoch = self.epoch.read().unwrap();
            (epoch.time, epoch.index as usize)
        };
        let (second, first) = self.slots.split_at(index);
        let iter = first.iter().chain(second.iter());
        for (n, slot) in iter.enumerate() {
            let deadline = { slot.read().unwrap().last().map(|timer| timer.deadline) };
            if let Some(deadline) = deadline {
                let ns_since_epoch = u64::from(deadline) + (n as u64 * u64::from(NS_PER_SLOT));
                let deadline = epoch_time + Duration::from_nanos(ns_since_epoch);
                return Some(deadline);
            }
        }

        #[rustfmt::skip]
        self.overflow.read().unwrap().last().map(|timer| timer.deadline)
    }

    /// Add a new deadline.
    pub(crate) fn add(&self, pid: ProcessId, deadline: Instant) {
        // NOTE: it's possible that we call `add_timer` based on an outdated
        // epoch.
        self.get_timers(pid, deadline, add_timer, add_timer);
    }

    /// Remove a previously added deadline.
    pub(crate) fn remove(&self, pid: ProcessId, deadline: Instant) {
        self.get_timers(pid, deadline, remove_timer, remove_timer);
    }

    /// Change the `ProcessId` of a previously added deadline.
    pub(crate) fn change(&self, pid: ProcessId, deadline: Instant, new_pid: ProcessId) {
        self.get_timers(
            pid,
            deadline,
            |timers, timer| change_timer(timers, timer, new_pid),
            |timers, timer| change_timer(timers, timer, new_pid),
        );
    }

    /// Determines in what list of timers a timer with `pid` and `deadline`
    /// would be/go into. Then calls the `slot_f` function for a timer list in
    /// the slots, or `overflow_f` with the overflow list.
    fn get_timers<SF, OF>(&self, pid: ProcessId, deadline: Instant, slot_f: SF, overflow_f: OF)
    where
        SF: FnOnce(&mut Vec<Timer<TimeOffset>>, Timer<TimeOffset>),
        OF: FnOnce(&mut Vec<Timer<Instant>>, Timer<Instant>),
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
            slot_f(&mut timers, Timer { pid, deadline });
        } else {
            // Too far into the future to fit in the slots.
            let mut overflow = self.overflow.write().unwrap();
            overflow_f(&mut overflow, Timer { pid, deadline });
        }
    }

    /// Remove the next deadline that passed `now` returning the pid.
    ///
    /// # Safety
    ///
    /// `now` may never go backwards between calls.
    pub(crate) fn remove_next(&self, now: Instant) -> Option<ProcessId> {
        loop {
            // NOTE: Each loop iteration needs to calculate the `epoch_offset`
            // as the epoch changes each iteration.
            let (epoch_time, index) = {
                let epoch = self.epoch.read().unwrap();
                (epoch.time, epoch.index as usize)
            };
            // Safety: `now` can't go backwards, otherwise this will panic.
            let epoch_offset = now.duration_since(epoch_time).as_nanos();
            // NOTE: this truncates, which is fine as we need a max. of
            // `NS_PER_SLOT` anyway.
            #[allow(clippy::cast_possible_truncation)]
            let epoch_offset = min(epoch_offset, u128::from(TimeOffset::MAX)) as TimeOffset;
            let res = {
                let mut timers = self.slots[index].write().unwrap();
                remove_if_before(&mut timers, epoch_offset)
            };
            match res {
                Ok(timer) => return Some(timer.pid),
                Err(true) => {
                    // Safety: slot is empty, which makes calling
                    // `maybe_update_epoch` OK.
                    if !self.maybe_update_epoch(now) {
                        // Didn't update epoch, no more timers to process.
                        return None;
                    }
                    // Else try again in the next loop.
                }
                // Slot has timers with a deadline past `now`.
                Err(false) => return None,
            }
        }
    }

    /// Attempt to update the epoch based on the current time.
    ///
    /// # Panics
    ///
    /// This panics if the current slot is not empty.
    #[allow(clippy::cast_possible_truncation)] // TODO: move to new `epoch.index` line.
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
            self.add(timer.pid, timer.deadline);
        }
        true
    }
}

/// Add `timer` to `timers`, ensuring it remains sorted.
fn add_timer<T>(timers: &mut Vec<Timer<T>>, timer: Timer<T>)
where
    Timer<T>: Ord + Copy,
{
    let idx = timers.binary_search(&timer).into_ok_or_err();
    timers.insert(idx, timer);
}

/// Remove a previously added `timer` from `timers`, ensuring it remains sorted.
fn remove_timer<T>(timers: &mut Vec<Timer<T>>, timer: Timer<T>)
where
    Timer<T>: Ord + Copy,
{
    if let Ok(idx) = timers.binary_search(&timer) {
        let _ = timers.remove(idx);
    }
}

/// Change the pid of a previously added `timer` in `timers`
fn change_timer<T>(timers: &mut Vec<Timer<T>>, timer: Timer<T>, new_pid: ProcessId)
where
    Timer<T>: Ord + Copy,
{
    match timers.binary_search(&timer) {
        Ok(idx) => timers[idx].pid = new_pid,
        #[rustfmt::skip]
        Err(idx) => timers.insert(idx, Timer { pid: new_pid, deadline: timer.deadline }),
    }
}

/// Remove the first timer if it's before `time`.
///
/// Returns `Ok(timer)` if there is a timer with a deadline before `time`.
/// Returns `Err(is_empty)`, indicating if `timers` is empty. Returns
/// `Err(true)` is `timers` is empty, `Err(false)` if the are more timers in
/// `timers`, but none with a deadline before `time`.
fn remove_if_before<T>(timers: &mut Vec<Timer<T>>, time: T) -> Result<Timer<T>, bool>
where
    T: Ord + Copy,
{
    match timers.last() {
        // TODO: is the `unwrap` removed here? Or do we need `unwrap_unchecked`?
        Some(timer) if timer.deadline <= time => Ok(timers.pop().unwrap()),
        Some(_) => Err(false),
        None => Err(true),
    }
}

/// A timer.
///
/// # Notes
///
/// The [`Ord`] implementation is in reverse order, i.e. the deadline to expire
/// first will have the highest ordering value. Furthermore the ordering is only
/// done base on the deadline, the process id is ignored in ordering. This
/// allows `change_timer` to not worry about order when changing the process id
/// of a timer.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
struct Timer<T> {
    pid: ProcessId,
    deadline: T,
}

impl<T> Ord for Timer<T>
where
    T: Ord,
{
    fn cmp(&self, other: &Self) -> Ordering {
        other.deadline.cmp(&self.deadline)
    }
}

impl<T> PartialOrd for Timer<T>
where
    T: Ord,
{
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
