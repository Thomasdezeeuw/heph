//! Module with the local timers implementation.

// TODO:
// Benchmark:
//  * Number of slots (`SLOTS`).
//  * Milliseconds per slot (`MS_PER_SLOT`).
//  * BinaryHeap vs sorted Vec.
//  * Making `Timer` `Copy`.

use std::cmp::Ordering;
use std::collections::binary_heap::{BinaryHeap, PeekMut};
use std::mem::size_of;
use std::time::{Duration, Instant};

use crate::rt::ProcessId;

// Gives use 64 * 200 ms = 12,8 seconds for the wheel.
const SLOTS: usize = 1 << 6;
const MS_PER_SLOT: TimeOffset = 200;
const OVERFLOW: Duration = Duration::from_millis(SLOTS as u64 * MS_PER_SLOT as u64);

/// Time offset since the epoch of [`Timers::epoch`].
///
/// Must fit [`MS_PER_SLOT`].
type TimeOffset = u8;

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
                        let time_offset = timer.time as u64 + (n as u64 * MS_PER_SLOT as u64);
                        let deadline = self.epoch + Duration::from_millis(time_offset);
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
        self.cached_next_deadline.update(deadline);

        let time_offset = ms_since(self.epoch, deadline);
        let index_offset = (time_offset / MS_PER_SLOT as u64) as usize;
        if index_offset > self.slots.len() {
            // Too far into the future.
            self.overflow.push(Timer {
                pid,
                time: deadline,
            });
            return;
        }

        let index = (self.index as usize + index_offset) % self.slots.len();
        let time = ms_as_offset(time_offset % MS_PER_SLOT as u64);
        log::error!(
            "adding timer: index={}, index_offset={}, time={}, time_offset={}",
            index,
            index_offset,
            time,
            time_offset
        );
        self.slots[index].push(Timer { pid, time });
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
            // NOTE: Each loop iteration needs to calculate the `time_offset` as
            // the epoch changes each iteration.
            let time_offset = ms_since(self.epoch, now);
            // Truncate the time offset to fit in `TimeOffset`.
            let time_offset = ms_as_offset(time_offset);
            match remove_if_after(self.current_slot(), time_offset) {
                Some(timer) => {
                    // Since we've just removed the first timer, invalid the
                    // cache.
                    self.cached_next_deadline = CachedInstant::Unset;
                    return Some(timer.pid);
                }
                None => {
                    // NOTE: we only reach this if we get `None` above, which
                    // means the current slot is empty, which makes calling
                    // `maybe_update_epoch` OK.
                    if !self.maybe_update_epoch(time_offset) {
                        // Didn't update epoch, no more timers to process.
                        return None;
                    }
                }
            }
        }
    }

    /// Attempt to update the epoch based on the current time.
    ///
    /// # Panics
    ///
    /// This panics if the current slot is not empty.
    fn maybe_update_epoch(&mut self, now_offset: TimeOffset) -> bool {
        if now_offset <= MS_PER_SLOT {
            // Can't move to the next slot yet.
            return false;
        }
        debug_assert!(self.current_slot().is_empty());

        // Move to the next slot and update the epoch.
        self.index = (self.index + 1) % self.slots.len() as u8;
        self.epoch += Duration::from_millis(MS_PER_SLOT as u64);

        // Next move all the overflow timers that now fit in the slots.
        let time = self.epoch + OVERFLOW;
        while let Some(timer) = remove_if_after(&mut self.overflow, time) {
            self.cached_next_deadline.update(timer.time);
            let time_offset = ms_since(self.epoch, timer.time);
            // NOTE: because we only remove timers that fit in the slot we don't
            // check the `index_offset` here.
            let index_offset = (time_offset / MS_PER_SLOT as u64) as usize;
            let index = (self.index as usize + index_offset) % self.slots.len();
            let time = ms_as_offset(time_offset % MS_PER_SLOT as u64);
            self.slots[index].push(Timer {
                pid: timer.pid,
                time,
            });
        }

        true
    }

    fn current_slot(&mut self) -> &mut BinaryHeap<Timer<TimeOffset>> {
        // Safety: `self.index` is always valid.
        &mut self.slots[self.index as usize]
    }
}

/// Truncates `millis` to [`TimeOffset`].
fn ms_as_offset(millis: u64) -> TimeOffset {
    const MASK: u64 = (1 << (size_of::<TimeOffset>() * 8)) - 1;
    debug_assert!(millis < u64::MAX);
    (millis & MASK) as TimeOffset
}

/// Returns the number of milliseconds between `now` and `epoch`, truncated to
/// fit in an `u64`.
fn ms_since(epoch: Instant, now: Instant) -> u64 {
    const MASK: u128 = (1 << (size_of::<u64>() * 8)) - 1;
    let millis = now.duration_since(epoch).as_millis();
    debug_assert!(millis < u64::MAX as u128);
    (millis & MASK) as u64
}

/// Remove the first timer after `time`.
fn remove_if_after<T>(timers: &mut BinaryHeap<Timer<T>>, time: T) -> Option<Timer<T>>
where
    T: Ord,
{
    match timers.peek_mut() {
        Some(timer) if timer.time <= time => Some(PeekMut::pop(timer)),
        Some(_) | None => None,
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
