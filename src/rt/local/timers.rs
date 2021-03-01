//! Module with the local timers implementation.

// TODO:
// Benchmark:
//  * Number of slots (`SLOTS`).
//  * Nanoseconds per slot (`NS_PER_SLOT`).
//  * BinaryHeap vs sorted Vec.
//  * Making `Timer` `Copy`.

use std::cmp::Ordering;
use std::collections::binary_heap::{BinaryHeap, PeekMut};
use std::time::{Duration, Instant};

use crate::rt::ProcessId;

/// Bits needed for the number of slots.
const SLOT_BITS: usize = 6;
/// Number of slots in the [`Timers`] wheel, 32.
const SLOTS: usize = 1 << SLOT_BITS;
/// Bits needed for the nanoseconds per slot.
const NS_PER_SLOT_BITS: usize = 30;
/// Nanoseconds per slot, 1073741824 ns ~= 1073 milliseconds.
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
// TODO: doc.
#[derive(Debug)]
pub(crate) struct Timers {
    /// Current epoch.
    epoch: Instant,
    /// Current index into `slots`.
    index: u8,
    slots: [BinaryHeap<Timer<TimeOffset>>; SLOTS],
    overflow: BinaryHeap<Timer<Instant>>,
    cached_next_deadline: CachedInstant,
}

impl Timers {
    /// Create a new collection of timers.
    pub(crate) fn new() -> Timers {
        Timers {
            epoch: Instant::now(),
            index: 0,
            // TODO: replace with constant once `BinaryHeap::new` is constant.
            #[rustfmt::skip]
            slots: [BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new(), BinaryHeap::new()],
            overflow: BinaryHeap::new(),
            cached_next_deadline: CachedInstant::Empty,
        }
    }

    /// Returns the next deadline, if any.
    pub(crate) fn next(&mut self) -> Option<Instant> {
        match self.cached_next_deadline {
            CachedInstant::Empty => None,
            CachedInstant::Set(deadline) => Some(deadline),
            CachedInstant::Unset => {
                let (first, second) = self.slots.split_at(self.index as usize);
                let iter = first.iter().chain(second.iter());
                for (n, slot) in iter.enumerate() {
                    if let Some(timer) = slot.peek() {
                        let ns_since_epoch = timer.time as u64 + (n as u64 * NS_PER_SLOT as u64);
                        let deadline = self.epoch + Duration::from_nanos(ns_since_epoch);
                        self.cached_next_deadline = CachedInstant::Set(deadline);
                        return Some(deadline);
                    }
                }

                self.overflow
                    .peek()
                    .map(|timer| timer.time)
                    .map(|deadline| {
                        self.cached_next_deadline = CachedInstant::Set(deadline);
                        deadline
                    })
            }
        }
    }

    /// Add a new deadline.
    pub(crate) fn add(&mut self, pid: ProcessId, deadline: Instant) {
        // Update the cache.
        self.cached_next_deadline.update(deadline);

        let ns_since_epoch = deadline.duration_since(self.epoch).as_nanos();
        if ns_since_epoch > NS_OVERFLOW as u128 {
            // Too far into the future to fit in the slots.
            self.overflow.push(Timer {
                pid,
                time: deadline,
            });
        } else {
            // TODO: doc this.
            let time = (ns_since_epoch & NS_SLOT_MASK) as TimeOffset;
            let index = ((ns_since_epoch >> NS_PER_SLOT_BITS) & ((1 << SLOT_BITS) - 1)) as usize;
            self.slots[index].push(Timer { pid, time });
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
    pub(crate) fn remove_next(&mut self, now: Instant) -> Option<ProcessId> {
        loop {
            // NOTE: Each loop iteration needs to calculate the `epoch_offset`
            // as the epoch changes each iteration.
            let epoch_offset = now.duration_since(self.epoch).as_nanos();
            // NOTE: this truncates, which is fine as we need max of
            // `NS_PER_SLOT` anyway.
            let epoch_offset = (epoch_offset & NS_SLOT_MASK) as TimeOffset;
            match remove_if_after(self.current_slot(), epoch_offset) {
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
    fn maybe_update_epoch(&mut self, epoch_offset: TimeOffset) -> bool {
        if epoch_offset < NS_PER_SLOT {
            // Can't move to the next slot yet.
            return false;
        }
        debug_assert!(self.current_slot().is_empty());

        // The index of the last slot, after we update the epoch below.
        let last_index = self.index as usize;
        // Move to the next slot and update the epoch.
        self.index = (self.index + 1) % self.slots.len() as u8;
        self.epoch += DURATION_PER_SLOT;

        // Next move all the overflow timers that now fit in the new slot (the
        // slot that was previously emptied).
        let time = self.epoch + OVERFLOW_DURATION;
        while let Ok(timer) = remove_if_after(&mut self.overflow, time) {
            // NOTE: We know two things:
            // 1) all timers in the overflow previously didn't fit in
            //    the slots,
            // 2) whenever we update the epoch we move all timers that fit into
            //    the new slot.
            // Based on this we know that we will only get timers that fit in
            // the new slot (the slot we've just emptied), hence we can safely
            // call `as_offset` as it won't truncate and all the timers will fit
            // in the slot with `last_index`.
            self.slots[last_index].push(Timer {
                pid: timer.pid,
                time: as_offset(self.epoch, timer.time),
            });
        }

        true
    }

    fn current_slot(&mut self) -> &mut BinaryHeap<Timer<TimeOffset>> {
        // Safety: `self.index` is always valid.
        &mut self.slots[self.index as usize]
    }
}

/// Returns the different between `epoch` and `time`, truncated to
/// [`TimeOffset`].
fn as_offset(epoch: Instant, time: Instant) -> TimeOffset {
    let nanos = time.duration_since(epoch).as_nanos();
    debug_assert!(nanos < NS_PER_SLOT as u128);
    (nanos & NS_SLOT_MASK) as TimeOffset
}

/// Remove the first timer if it's before `time`.
///
/// Returns `Ok(timer)` if there is a timer with a deadline before `time`.
/// Returns `Err(is_empty)`, indicating if `timers` is empty. Returns
/// `Err(true)` is `timers` is empty, `Err(false)` if the are more timers in
/// `timers`, but none with a deadline before `time`.
fn remove_if_after<T>(timers: &mut BinaryHeap<Timer<T>>, time: T) -> Result<Timer<T>, bool>
where
    T: Ord,
{
    match timers.peek_mut() {
        Some(timer) if timer.time <= time => Ok(PeekMut::pop(timer)),
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
}

/// A timer.
///
/// # Notes
///
/// The [`Ord`] implementation is in reverse to max [`BinaryHeap`] a min-heap.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Timer<T> {
    pid: ProcessId,
    time: T,
}

impl<T> Ord for Timer<T>
where
    T: Ord,
{
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .time
            .cmp(&self.time)
            .then_with(|| other.pid.cmp(&self.pid))
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
