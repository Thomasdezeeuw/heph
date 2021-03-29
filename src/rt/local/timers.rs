//! Module with the local timers implementation.
//!
//! Also see the [shared timers implementation].
//!
//! [shared timers implementation]: crate::rt::shared::timers

use std::cmp::{min, Ordering};
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
/// Must fit [`MS_PER_SLOT`].
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
/// `index` by 1 and the `epoch` by [`NS_PER_SLOT`] nanoseconds. This means the
/// next slot (now `slots[index+1]`) holds timers that expire `0..NS_PER_SLOT`
/// nanoseconds after `epoch`.
#[derive(Debug)]
pub(crate) struct Timers {
    /// Current epoch.
    epoch: Instant,
    /// Current index into `slots`.
    index: u8,
    /// The vectors are sorted.
    slots: [Vec<Timer<TimeOffset>>; SLOTS],
    /// The vector is sorted.
    overflow: Vec<Timer<Instant>>,
    /// Cache for the next deadline to expire.
    ///
    /// If `Timers` is empty this prevents us from checking all `slots` and the
    /// `overflow` list.
    cached_next_deadline: CachedInstant,
}

impl Timers {
    /// Create a new collection of timers.
    pub(crate) fn new() -> Timers {
        const EMPTY: Vec<Timer<TimeOffset>> = Vec::new();
        Timers {
            epoch: Instant::now(),
            index: 0,
            slots: [EMPTY; SLOTS],
            overflow: Vec::new(),
            cached_next_deadline: CachedInstant::Empty,
        }
    }

    /// Returns the next deadline, if any.
    pub(crate) fn next(&mut self) -> Option<Instant> {
        match self.cached_next_deadline {
            CachedInstant::Empty => None,
            CachedInstant::Set(deadline) => Some(deadline),
            CachedInstant::Unset => {
                let (second, first) = self.slots.split_at(self.index as usize);
                let iter = first.iter().chain(second.iter());
                for (n, slot) in iter.enumerate() {
                    if let Some(timer) = slot.last() {
                        let ns_since_epoch =
                            u64::from(timer.deadline) + (n as u64 * u64::from(NS_PER_SLOT));
                        let deadline = self.epoch + Duration::from_nanos(ns_since_epoch);
                        self.cached_next_deadline = CachedInstant::Set(deadline);
                        return Some(deadline);
                    }
                }

                if let Some(timer) = self.overflow.last() {
                    self.cached_next_deadline = CachedInstant::Set(timer.deadline);
                    Some(timer.deadline)
                } else {
                    self.cached_next_deadline = CachedInstant::Empty;
                    None
                }
            }
        }
    }

    /// Add a new deadline.
    pub(crate) fn add(&mut self, pid: ProcessId, deadline: Instant) {
        let deadline = self.checked_deadline(deadline);
        self.cached_next_deadline.update(deadline);
        self.get_timers(pid, deadline, add_timer, add_timer);
    }

    /// Remove a previously added deadline.
    pub(crate) fn remove(&mut self, pid: ProcessId, deadline: Instant) {
        let deadline = self.checked_deadline(deadline);
        self.cached_next_deadline.invalidate(deadline);
        self.get_timers(pid, deadline, remove_timer, remove_timer);
    }

    /// Change the `ProcessId` of a previously added deadline.
    pub(crate) fn change(&mut self, pid: ProcessId, deadline: Instant, new_pid: ProcessId) {
        let deadline = self.checked_deadline(deadline);
        // NOTE: we need to update the cache in the case where the deadline was
        // never added or expired, because the `timers` module depends on the
        // fact it will be scheduled once the timer expires.
        self.cached_next_deadline.update(deadline);
        // NOTE: don't need to update the change as it only keep track of the
        // deadline, which doesn't change.
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
    fn get_timers<SF, OF>(&mut self, pid: ProcessId, deadline: Instant, slot_f: SF, overflow_f: OF)
    where
        SF: FnOnce(&mut Vec<Timer<TimeOffset>>, Timer<TimeOffset>),
        OF: FnOnce(&mut Vec<Timer<Instant>>, Timer<Instant>),
    {
        let ns_since_epoch = deadline.saturating_duration_since(self.epoch).as_nanos();
        if ns_since_epoch < u128::from(NS_OVERFLOW) {
            #[allow(clippy::cast_possible_truncation)] // Truncation is OK.
            let offset = (ns_since_epoch & NS_SLOT_MASK) as TimeOffset;
            let index = ((ns_since_epoch >> NS_PER_SLOT_BITS) & ((1 << SLOT_BITS) - 1)) as usize;
            #[rustfmt::skip]
            debug_assert_eq!(
                deadline,
                self.epoch + Duration::from_nanos((index as u64 * u64::from(NS_PER_SLOT)) + u64::from(offset))
            );
            let index = (self.index as usize + index) % SLOTS;
            let timer = Timer {
                pid,
                deadline: offset,
            };
            slot_f(&mut self.slots[index], timer);
        } else {
            // Too far into the future to fit in the slots.
            overflow_f(&mut self.overflow, Timer { pid, deadline });
        }
    }

    /// Returns all deadlines that have expired (i.e. deadline < `now`).
    pub(crate) fn deadlines(&mut self, now: Instant) -> Deadlines<'_> {
        Deadlines { timers: self, now }
    }

    /// Remove the next deadline that passed `now` returning the pid.
    ///
    /// # Safety
    ///
    /// `now` may never go backwards between calls.
    fn remove_next(&mut self, now: Instant) -> Option<ProcessId> {
        loop {
            // NOTE: Each loop iteration needs to calculate the `epoch_offset`
            // as the epoch changes each iteration.
            let epoch_offset = now.duration_since(self.epoch).as_nanos();
            #[allow(clippy::cast_possible_truncation)]
            let epoch_offset = min(epoch_offset, u128::from(TimeOffset::MAX)) as TimeOffset;
            match remove_if_before(self.current_slot(), epoch_offset) {
                Ok(timer) => {
                    // Since we've just removed the first timer, invalid the
                    // cache.
                    self.cached_next_deadline = CachedInstant::Unset;
                    return Some(timer.pid);
                }
                Err(true) => {
                    // Safety: slot is empty, which makes calling
                    // `maybe_update_epoch` OK.
                    if !self.maybe_update_epoch(epoch_offset) {
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
    #[allow(clippy::cast_possible_truncation)] // TODO: move to new `self.index` line.
    fn maybe_update_epoch(&mut self, epoch_offset: TimeOffset) -> bool {
        if epoch_offset < NS_PER_SLOT {
            // Can't move to the next slot yet.
            return false;
        }
        debug_assert!(self.current_slot().is_empty());

        // The index of the last slot, after we update the epoch below.
        let last_index = self.index as usize;
        // Move to the next slot and update the epoch.
        self.index = (self.index + 1) % SLOTS as u8;
        self.epoch += DURATION_PER_SLOT;

        // Next move all the overflow timers that now fit in the new slot (the
        // slot that was previously emptied).
        let time = self.epoch + OVERFLOW_DURATION;
        let slot_epoch = self.epoch + OVERFLOW_DURATION - DURATION_PER_SLOT;
        let timers = &mut self.slots[last_index];
        while let Ok(timer) = remove_if_before(&mut self.overflow, time) {
            // We add the timers in reverse order here as we remove the timer
            // first to expire from overflow first.
            timers.push(Timer {
                pid: timer.pid,
                deadline: as_offset(slot_epoch, timer.deadline),
            });
        }
        // At this point the timer first to expire is the first timer, but it
        // needs to be the last. So we reverse the order, which ensures the list
        // is sorted again.
        timers.reverse();
        debug_assert!(timers.is_sorted());

        true
    }

    /// Returns the `deadline` that can safely be added to the timers. Any
    /// deadline before the current epoch is set to the current epoch.
    fn checked_deadline(&self, deadline: Instant) -> Instant {
        if deadline < self.epoch {
            self.epoch
        } else {
            deadline
        }
    }

    fn current_slot(&mut self) -> &mut Vec<Timer<TimeOffset>> {
        // Safety: `self.index` is always valid.
        &mut self.slots[self.index as usize]
    }
}

/// Add `timer` to `timers`, ensuring it remains sorted.
fn add_timer<T>(timers: &mut Vec<Timer<T>>, timer: Timer<T>)
where
    Timer<T>: Ord,
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

/// Returns the different between `epoch` and `time`, truncated to
/// [`TimeOffset`].
#[allow(clippy::cast_possible_truncation)] // TODO: move to last line.
fn as_offset(epoch: Instant, time: Instant) -> TimeOffset {
    let nanos = time.duration_since(epoch).as_nanos();
    debug_assert!(nanos < u128::from(NS_PER_SLOT));
    (nanos & NS_SLOT_MASK) as TimeOffset
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

/// To avoid having to check all slots and the overflow for timers in an
/// [`Timers`] this type caches the earliest deadline. This speeds up
/// [`Timers::next`].
#[derive(Debug)]
enum CachedInstant {
    /// [`Timers`] is empty.
    Empty,
    /// Was previously set, but has elapsed.
    /// This is different from `Empty` as it means there *might* be a timer in
    /// [`Timers`].
    Unset,
    /// The next deadline.
    Set(Instant),
}

impl CachedInstant {
    /// Update the cached instant with `deadline`.
    fn update(&mut self, deadline: Instant) {
        match self {
            CachedInstant::Empty => *self = CachedInstant::Set(deadline),
            CachedInstant::Unset => {
                // Can't set the instant here as we don't know if there are
                // earlier deadlines in the [`Timers`] struct.
            }
            CachedInstant::Set(current) if deadline < *current => {
                // `deadline` is earlier, so we update it.
                *current = deadline;
            }
            CachedInstant::Set(_) => { /* Current deadline is earlier. */ }
        }
    }

    /// Invalidate the the cache if the current deadline is equal to `deadline`.
    fn invalidate(&mut self, deadline: Instant) {
        match self {
            CachedInstant::Set(current) if *current == deadline => {
                *self = CachedInstant::Unset;
            }
            CachedInstant::Set(_) | CachedInstant::Empty | CachedInstant::Unset => {}
        }
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

/// Returns all timers that have passed (since the iterator was created).
#[derive(Debug)]
pub(crate) struct Deadlines<'t> {
    timers: &'t mut Timers,
    now: Instant,
}

impl<'t> Iterator for Deadlines<'t> {
    type Item = ProcessId;

    fn next(&mut self) -> Option<Self::Item> {
        self.timers.remove_next(self.now)
    }
}
