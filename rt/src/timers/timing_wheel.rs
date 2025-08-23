//! This [`Timers`] implementation is based on a Timing Wheel as discussed in
//! the paper "Hashed and hierarchical timing wheels: efficient data structures
//! for implementing a timer facility" by George Varghese and Anthony Lauck
//! (1997).
//!
//! This uses a scheme that splits the timers based on when they're going to
//! expire. It has 64 ([`SLOTS`]) slots each representing roughly a second of
//! time ([`NS_PER_SLOT`]). This allows us to only consider a portion of all
//! timers when processing the timers. Any timers that don't fit into these
//! slots, i.e. timers with a deadline more than 68 seconds ([`NS_OVERFLOW`])
//! past `epoch`, are put in a overflow list. Ideally this overflow list is
//! empty however.
//!
//! The `slots` hold the timers with a [`TimeOffset`] which is the number of
//! nanosecond since epoch times it's index. The `index` field determines the
//! current zero-slot, meaning its timers will expire next and all have a
//! deadline within `0..NS_PER_SLOT` nanoseconds after `epoch`. The
//! `slots[index+1]` list will have timers that expire
//! `NS_PER_SLOT..2*NS_PER_SLOT` nanoseconds after `epoch`. In other words each
//! slot holds the timers that expire in the ~second after the previous slot.
//!
//! Whenever timers are expired by `expire_timers` it will attempt to update the
//! `epoch`, which is used as anchor point to determine in what slot/overflow
//! the timer must go (see above). When updating the epoch it will increase the
//! `index` by 1 and the `epoch` by [`NS_PER_SLOT`] nanoseconds. This means the
//! next slot (now `slots[index+1]`) holds timers that expire `0..NS_PER_SLOT`
//! nanoseconds after `epoch`.
//!
//! Note that for the `shared` version, which uses the same implementation as
//! described above, it's possible for a thread to read the epoch (index and
//! time), than gets descheduled, another thread updates the epoch and finally
//! the second thread insert a timer based on a now outdated epoch. This
//! situation is fine as the timer will still be added to the correct slot, but
//! it has a higher change of being added to the overflow list (which
//! `maybe_update_epoch` deals with correctly).

use std::cmp::{max, min};
use std::sync::RwLock;
use std::task;
use std::time::{Duration, Instant};

use crate::timers::TimerToken;

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

/// Thread-local timing wheel implementation.
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

/// A timer in [`Timers`].
#[derive(Debug)]
struct Timer<T> {
    deadline: T,
    waker: task::Waker,
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

    /// Returns the total number of timers.
    pub(crate) fn len(&self) -> usize {
        let mut timers = 0;
        for slots in &self.slots {
            timers += slots.len();
        }
        timers + self.overflow.len()
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

    /// Same as [`next`], but returns a [`Duration`] instead. If the next
    /// deadline is already passed this returns a duration of zero.
    ///
    /// [`next`]: Timers::next
    pub(crate) fn next_timer(&mut self) -> Option<Duration> {
        self.next().map(|deadline| {
            Instant::now()
                .checked_duration_since(deadline)
                .unwrap_or(Duration::ZERO)
        })
    }

    /// Add a new deadline.
    pub(crate) fn add(&mut self, deadline: Instant, waker: task::Waker) -> TimerToken {
        // Can't have deadline before the epoch, so we'll add a deadline with
        // same time as the epoch instead.
        let deadline = max(deadline, self.epoch);
        self.cached_next_deadline.update(deadline);
        self.get_timers(deadline, |timers| match timers {
            TimerLocation::InSlot((timers, deadline)) => add_timer(timers, deadline, waker),
            TimerLocation::Overflow((timers, deadline)) => add_timer(timers, deadline, waker),
        })
    }

    /// Remove a previously added deadline.
    pub(crate) fn remove(&mut self, deadline: Instant, token: TimerToken) {
        let deadline = max(deadline, self.epoch);
        self.cached_next_deadline.invalidate(deadline);
        self.get_timers(deadline, |timers| match timers {
            TimerLocation::InSlot((timers, deadline)) => remove_timer(timers, deadline, token),
            TimerLocation::Overflow((timers, deadline)) => remove_timer(timers, deadline, token),
        });
    }

    /// Determines in what list of timers a timer with `pid` and `deadline`
    /// would be/go into. Then calls the `slot_f` function for a timer list in
    /// the slots, or `overflow_f` with the overflow list.
    fn get_timers<F, T>(&mut self, deadline: Instant, f: F) -> T
    where
        F: FnOnce(TimerLocation<'_>) -> T,
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
            f(TimerLocation::InSlot((&mut self.slots[index], offset)))
        } else {
            // Too far into the future to fit in the slots.
            f(TimerLocation::Overflow((&mut self.overflow, deadline)))
        }
    }

    /// Expire all timers that have elapsed based on `now`. Returns the amount
    /// of expired timers.
    ///
    /// # Safety
    ///
    /// `now` may never go backwards between calls.
    pub(crate) fn expire_timers(&mut self, now: Instant) -> usize {
        let mut amount = 0;
        self.cached_next_deadline = CachedInstant::Unset;
        loop {
            // NOTE: Each loop iteration needs to calculate the `epoch_offset`
            // as the epoch changes each iteration.
            let epoch_offset = now.duration_since(self.epoch).as_nanos();
            #[allow(clippy::cast_possible_truncation)]
            let epoch_offset = min(epoch_offset, u128::from(TimeOffset::MAX)) as TimeOffset;
            let slot = self.current_slot();
            loop {
                match remove_if_before(slot, epoch_offset) {
                    Ok(timer) => {
                        timer.waker.wake();
                        amount += 1;
                        // Try another timer in this slot.
                        continue;
                    }
                    Err(true) => {
                        // SAFETY: slot is empty, which makes calling
                        // `maybe_update_epoch` OK.
                        if !self.maybe_update_epoch(epoch_offset) {
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
    #[allow(clippy::debug_assert_with_mut_call, clippy::cast_possible_truncation)]
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
        let slot_epoch = self.epoch + (OVERFLOW_DURATION - DURATION_PER_SLOT);
        let timers = &mut self.slots[last_index];
        while let Ok(timer) = remove_if_before(&mut self.overflow, time) {
            // We add the timers in reverse order here as we remove the timer
            // first to expire from overflow first.
            timers.push(Timer {
                deadline: as_offset(slot_epoch, timer.deadline),
                waker: timer.waker,
            });
        }
        // At this point the timer first to expire is the first timer, but it
        // needs to be the last. So we reverse the order, which ensures the list
        // is sorted again.
        timers.reverse();
        debug_assert!(timers.is_sorted_by(|t1, t2| t1.deadline <= t2.deadline));

        true
    }

    fn current_slot(&mut self) -> &mut Vec<Timer<TimeOffset>> {
        // SAFETY: `self.index` is always valid.
        &mut self.slots[self.index as usize]
    }
}

/// Thread-safe timing wheel implementation.
#[derive(Debug)]
pub(crate) struct SharedTimers {
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

impl SharedTimers {
    /// Create a new collection of timers.
    pub(crate) fn new() -> SharedTimers {
        #[allow(clippy::declare_interior_mutable_const)]
        const EMPTY: RwLock<Vec<Timer<TimeOffset>>> = RwLock::new(Vec::new());
        SharedTimers {
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
    /// [`next`]: SharedTimers::next
    pub(crate) fn next_timer(&self) -> Option<Duration> {
        self.next().map(|deadline| {
            Instant::now()
                .checked_duration_since(deadline)
                .unwrap_or(Duration::ZERO)
        })
    }

    /// Add a new deadline.
    pub(crate) fn add(&self, deadline: Instant, waker: task::Waker) -> TimerToken {
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

/// Location of a timer.
enum TimerLocation<'a> {
    /// In of the wheel's slots.
    InSlot((&'a mut Vec<Timer<TimeOffset>>, TimeOffset)),
    /// In the overflow vector.
    Overflow((&'a mut Vec<Timer<Instant>>, Instant)),
}

/// Add a new timer to `timers`, ensuring it remains sorted.
fn add_timer<T: Ord>(timers: &mut Vec<Timer<T>>, deadline: T, waker: task::Waker) -> TimerToken {
    let idx = match timers.binary_search_by(|timer| timer.deadline.cmp(&deadline)) {
        Ok(idx) | Err(idx) => idx,
    };
    let token = TimerToken::for_waker(&waker);
    timers.insert(idx, Timer { deadline, waker });
    token
}

/// Remove a previously added `deadline` from `timers`, ensuring it remains sorted.
#[allow(clippy::needless_pass_by_value)]
fn remove_timer<T: Ord>(timers: &mut Vec<Timer<T>>, deadline: T, token: TimerToken) {
    if let Ok(idx) = timers.binary_search_by(|timer| timer.deadline.cmp(&deadline)) {
        if token.is_for_waker(&timers[idx].waker) {
            _ = timers.remove(idx);
        }
    }
}

/// Remove the first timer if it's before `time`.
///
/// Returns `Ok(timer)` if there is a timer with a deadline before `time`.
/// Otherwise this returns `Err(true)` if `timers` is empty or `Err(false)` if
/// the are more timers in `timers`, but none with a deadline before `time`.
#[allow(clippy::needless_pass_by_value)]
fn remove_if_before<T: Ord>(timers: &mut Vec<Timer<T>>, time: T) -> Result<Timer<T>, bool> {
    match timers.last() {
        Some(timer) if timer.deadline <= time => Ok(timers.pop().unwrap()),
        Some(_) => Err(false),
        None => Err(true),
    }
}

/// Returns the different between `epoch` and `time`, truncated to
/// [`TimeOffset`].
fn as_offset(epoch: Instant, time: Instant) -> TimeOffset {
    let nanos = time.duration_since(epoch).as_nanos();
    debug_assert!(nanos < u128::from(NS_PER_SLOT));
    (nanos & NS_SLOT_MASK) as TimeOffset
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
            CachedInstant::Set(current) if deadline < *current => {
                // `deadline` is earlier, so we update it.
                *current = deadline;
            }
            // Can't set the instant as we don't know if there are earlier
            // deadlines in the [`Timers`] struct.
            CachedInstant::Unset |
            // Current deadline is earlier.
            CachedInstant::Set(_) => {},
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::task::{Wake, Waker};
    use std::time::Duration;

    use crate::timers::timing_wheel::{TimerToken, Timers, DURATION_PER_SLOT, NS_PER_SLOT, SLOTS};

    struct WakerBuilder<const N: usize> {
        awoken: Arc<[AtomicBool; N]>,
        n: usize,
    }

    impl<const N: usize> WakerBuilder<N> {
        fn new() -> WakerBuilder<N> {
            const FALSE: AtomicBool = AtomicBool::new(false);
            WakerBuilder {
                awoken: Arc::new([FALSE; N]),
                n: 0,
            }
        }

        fn task_waker(&mut self) -> (usize, Waker) {
            let n = self.n;
            self.n += 1;
            assert!(n <= N, "created too many task::Wakers");
            (
                n,
                Waker::from(Arc::new(TaskWaker {
                    awoken: self.awoken.clone(),
                    n,
                })),
            )
        }

        fn is_awoken(&self, n: usize) -> bool {
            self.awoken[n].load(Ordering::Acquire)
        }
    }

    /// [`Wake`] implementation.
    struct TaskWaker<const N: usize> {
        awoken: Arc<[AtomicBool; N]>,
        n: usize,
    }

    impl<const N: usize> Wake for TaskWaker<N> {
        fn wake(self: Arc<Self>) {
            self.awoken[self.n].store(true, Ordering::Release)
        }
    }

    #[test]
    fn add_deadline_first_slot() {
        let mut timers = Timers::new();
        let mut wakers = WakerBuilder::<1>::new();

        let deadline = timers.epoch + Duration::from_millis(100);
        let (n, waker) = wakers.task_waker();
        _ = timers.add(deadline, waker);
        assert_eq!(timers.next(), Some(deadline));

        // Not yet expired.
        assert_eq!(timers.expire_timers(timers.epoch), 0);

        // Waker is called when the deadline is expired.
        assert_eq!(timers.expire_timers(deadline), 1);
        assert!(wakers.is_awoken(n));

        // No more timers.
        assert_eq!(timers.expire_timers(deadline), 0);
    }

    #[test]
    fn add_deadline_second_slot() {
        let mut timers = Timers::new();
        let mut wakers = WakerBuilder::<1>::new();

        let deadline = timers.epoch + Duration::from_nanos(NS_PER_SLOT as u64 + 10);
        let (n, waker) = wakers.task_waker();
        _ = timers.add(deadline, waker);
        assert_eq!(timers.next(), Some(deadline));

        assert_eq!(timers.expire_timers(timers.epoch), 0);
        assert_eq!(timers.index, 0);
        assert_eq!(timers.next(), Some(deadline));

        assert_eq!(timers.expire_timers(deadline), 1);
        assert!(wakers.is_awoken(n));
        assert_eq!(timers.index, 1);

        assert_eq!(timers.expire_timers(timers.epoch), 0);
    }

    #[test]
    fn add_deadline_overflow() {
        let mut timers = Timers::new();
        let mut wakers = WakerBuilder::<1>::new();

        let deadline = timers.epoch + Duration::from_nanos(SLOTS as u64 * NS_PER_SLOT as u64 + 10);
        let (n, waker) = wakers.task_waker();
        _ = timers.add(deadline, waker);
        assert_eq!(timers.next(), Some(deadline));

        assert_eq!(timers.expire_timers(timers.epoch), 0);
        assert_eq!(timers.index, 0);
        assert_eq!(timers.next(), Some(deadline));

        assert_eq!(timers.expire_timers(deadline), 1);
        assert!(wakers.is_awoken(n));
        // Should have advanced the epoch to come back around to 0.
        assert_eq!(timers.index, 0);

        assert_eq!(timers.expire_timers(timers.epoch), 0);
    }

    #[test]
    fn add_deadline_to_all_slots() {
        let mut timers = Timers::new();
        let mut wakers = WakerBuilder::<{ SLOTS + 1 }>::new();

        // Add a deadline to all slots and the overflow list.
        for n in 0..=SLOTS {
            let deadline =
                timers.epoch + Duration::from_nanos((n as u64 * NS_PER_SLOT as u64) + 10);
            let (n2, waker) = wakers.task_waker();
            assert_eq!(n, n2);
            _ = timers.add(deadline, waker);
        }

        let first_deadline = timers.epoch + Duration::from_nanos(10);
        assert_eq!(timers.next(), Some(first_deadline));
        assert_eq!(timers.expire_timers(timers.epoch), 0);
        assert_eq!(timers.index, 0);

        let mut expected_next_deadline = first_deadline;
        let mut expected_index = 0;
        for n in 0..=SLOTS {
            assert_eq!(timers.next(), Some(expected_next_deadline));
            let now = expected_next_deadline + Duration::from_nanos(1);
            assert_eq!(timers.expire_timers(now), 1);
            assert!(wakers.is_awoken(n));
            assert_eq!(timers.index, expected_index);

            assert_eq!(timers.expire_timers(timers.epoch), 0);
            assert_eq!(timers.index, expected_index);

            expected_index = (expected_index + 1) % SLOTS as u8;
            expected_next_deadline += DURATION_PER_SLOT;
        }
    }

    #[test]
    fn add_deadline_in_the_past() {
        let mut timers = Timers::new();
        let mut wakers = WakerBuilder::<1>::new();

        let (n, waker) = wakers.task_waker();
        _ = timers.add(timers.epoch - Duration::from_secs(1), waker);
        assert_eq!(timers.next(), Some(timers.epoch));

        assert_eq!(timers.expire_timers(timers.epoch), 1);
        assert!(wakers.is_awoken(n));
    }

    #[test]
    fn adding_earlier_deadline_updates_cache() {
        let mut timers = Timers::new();
        let mut wakers = WakerBuilder::<2>::new();

        let deadline1 = timers.epoch + Duration::from_secs(2);
        let (n1, waker) = wakers.task_waker();
        _ = timers.add(deadline1, waker);
        let deadline2 = timers.epoch + Duration::from_secs(1);
        let (n2, waker) = wakers.task_waker();
        _ = timers.add(deadline2, waker);
        assert_eq!(timers.next(), Some(deadline2));

        assert_eq!(timers.expire_timers(deadline1), 2);
        assert!(wakers.is_awoken(n1));
        assert!(wakers.is_awoken(n2));
        assert_eq!(timers.expire_timers(deadline1), 0);
    }

    #[test]
    fn remove_deadline() {
        let mut timers = Timers::new();
        let mut wakers = WakerBuilder::<1>::new();

        let deadline = timers.epoch + Duration::from_millis(10);
        let (n, waker) = wakers.task_waker();
        let token = timers.add(deadline, waker);
        timers.remove(deadline, token);
        assert_eq!(timers.next(), None);
        assert_eq!(timers.expire_timers(timers.epoch), 0);
        assert!(!wakers.is_awoken(n));
    }

    #[test]
    fn remove_never_added_deadline() {
        let mut timers = Timers::new();

        let deadline = timers.epoch + Duration::from_millis(10);
        assert_eq!(timers.next(), None);
        assert_eq!(timers.expire_timers(timers.epoch), 0);
        timers.remove(deadline, TimerToken(0));
        assert_eq!(timers.next(), None);
        assert_eq!(timers.expire_timers(timers.epoch), 0);
    }

    #[test]
    fn remove_expired_deadline() {
        let mut timers = Timers::new();
        let mut wakers = WakerBuilder::<1>::new();

        let deadline = timers.epoch + Duration::from_millis(10);
        let (n, waker) = wakers.task_waker();
        let token = timers.add(deadline, waker);

        assert_eq!(timers.next(), Some(deadline));
        assert_eq!(timers.expire_timers(deadline), 1);
        assert!(wakers.is_awoken(n));
        assert_eq!(timers.expire_timers(deadline), 0);

        timers.remove(deadline, token);
        assert_eq!(timers.next(), None);
        assert_eq!(timers.expire_timers(deadline), 0);
    }

    #[test]
    fn remove_deadline_from_all_slots() {
        let mut timers = Timers::new();
        let mut wakers = WakerBuilder::<{ SLOTS + 1 }>::new();

        // Add a deadline to all slots and the overflow list.
        let tokens: Vec<TimerToken> = (0..=SLOTS)
            .into_iter()
            .map(|n| {
                let deadline =
                    timers.epoch + Duration::from_nanos((n as u64 * NS_PER_SLOT as u64) + 10);
                let (n2, waker) = wakers.task_waker();
                assert_eq!(n2, n);
                timers.add(deadline, waker)
            })
            .collect();

        let first_deadline = timers.epoch + Duration::from_nanos(10);
        assert_eq!(timers.next(), Some(first_deadline));
        assert_eq!(timers.expire_timers(timers.epoch), 0);
        assert_eq!(timers.index, 0);

        let mut next_deadline = first_deadline;
        for (n, token) in tokens.into_iter().enumerate() {
            timers.remove(next_deadline, token);
            next_deadline += DURATION_PER_SLOT;

            if n == SLOTS {
                assert_eq!(timers.next(), None);
            } else {
                assert_eq!(timers.next(), Some(next_deadline));
            }
        }
    }

    #[test]
    fn remove_deadline_from_all_slots_interleaved() {
        let mut timers = Timers::new();
        let mut wakers = WakerBuilder::<{ SLOTS + 1 }>::new();

        // Add a deadline to all slots and the overflow list.
        for n in 0..=SLOTS {
            let deadline =
                timers.epoch + Duration::from_nanos((n as u64 * NS_PER_SLOT as u64) + 10);
            let (n2, waker) = wakers.task_waker();
            assert_eq!(n2, n);
            let token = timers.add(deadline, waker);
            timers.remove(deadline, token);
        }

        assert_eq!(timers.next(), None);
        assert_eq!(timers.expire_timers(timers.epoch), 0);
        assert_eq!(timers.index, 0);
    }

    #[test]
    fn remove_deadline_after_epoch_advance() {
        let mut timers = Timers::new();
        let mut wakers = WakerBuilder::<{ SLOTS + 1 }>::new();

        // Add a deadline to all slots and the overflow list.
        let tokens: Vec<TimerToken> = (0..=SLOTS)
            .into_iter()
            .map(|n| {
                let deadline =
                    timers.epoch + Duration::from_nanos((n as u64 * NS_PER_SLOT as u64) + 10);
                let (n2, waker) = wakers.task_waker();
                assert_eq!(n2, n);
                timers.add(deadline, waker)
            })
            .collect();

        let first_deadline = timers.epoch + Duration::from_nanos(10);
        let now = timers.epoch + DURATION_PER_SLOT;
        assert_eq!(timers.next(), Some(first_deadline));
        assert_eq!(timers.expire_timers(now), 1);
        assert!(wakers.is_awoken(0));
        assert_eq!(timers.expire_timers(timers.epoch), 0);
        assert_eq!(timers.index, 1);
        assert_eq!(timers.next(), Some(first_deadline + DURATION_PER_SLOT));

        let mut next_deadline = first_deadline + DURATION_PER_SLOT;
        for (n, token) in tokens.into_iter().skip(1).enumerate() {
            timers.remove(next_deadline, token);
            next_deadline += DURATION_PER_SLOT;

            if n == SLOTS - 1 {
                assert_eq!(timers.next(), None);
            } else {
                assert_eq!(timers.next(), Some(next_deadline));
            }
        }
    }

    #[test]
    fn remove_deadline_in_the_past() {
        let mut timers = Timers::new();
        let mut wakers = WakerBuilder::<1>::new();

        let deadline = timers.epoch - Duration::from_secs(1);
        let (n, waker) = wakers.task_waker();
        let token = timers.add(deadline, waker);
        assert_eq!(timers.next(), Some(timers.epoch));
        timers.remove(deadline, token);
        assert_eq!(timers.next(), None);
        assert_eq!(timers.expire_timers(timers.epoch), 0);
        assert!(!wakers.is_awoken(n));
    }

    mod shared {
        use std::time::Duration;

        use crate::timers::timing_wheel::tests::WakerBuilder;
        use crate::timers::timing_wheel::{
            SharedTimers, TimerToken, DURATION_PER_SLOT, NS_PER_SLOT, SLOTS,
        };

        #[test]
        fn add_deadline_first_slot() {
            let timers = SharedTimers::new();
            let mut wakers = WakerBuilder::<1>::new();
            let epoch = timers.epoch().0;
            let deadline = epoch + Duration::from_millis(100);

            let (n, waker) = wakers.task_waker();
            _ = timers.add(deadline, waker);
            assert_eq!(timers.next(), Some(deadline));

            // Not yet expired.
            assert_eq!(timers.expire_timers(epoch), 0);
            assert!(!wakers.is_awoken(n));

            // Waker is called when the deadline is expired.
            assert_eq!(timers.expire_timers(deadline), 1);
            assert!(wakers.is_awoken(n));

            // No more timers.
            assert_eq!(timers.expire_timers(deadline + Duration::from_secs(100)), 0);
        }

        #[test]
        fn add_deadline_second_slot() {
            let timers = SharedTimers::new();
            let mut wakers = WakerBuilder::<1>::new();
            let epoch = timers.epoch().0;

            let deadline = epoch + Duration::from_nanos(NS_PER_SLOT as u64 + 100);
            let (n, waker) = wakers.task_waker();
            _ = timers.add(deadline, waker);
            assert_eq!(timers.next(), Some(deadline));

            assert_eq!(timers.expire_timers(epoch), 0);
            assert_eq!(timers.epoch().1, 0);
            assert_eq!(timers.next(), Some(deadline));

            assert_eq!(timers.expire_timers(deadline), 1);
            assert!(wakers.is_awoken(n));

            assert_eq!(timers.epoch().1, 1);
            assert_eq!(timers.expire_timers(epoch), 0);
        }

        #[test]
        fn add_deadline_overflow() {
            let timers = SharedTimers::new();
            let mut wakers = WakerBuilder::<1>::new();
            let epoch = timers.epoch().0;

            let deadline = epoch + Duration::from_nanos(SLOTS as u64 * NS_PER_SLOT as u64 + 10);
            let (n, waker) = wakers.task_waker();
            _ = timers.add(deadline, waker);
            assert_eq!(timers.next(), Some(deadline));

            assert_eq!(timers.expire_timers(epoch), 0);
            assert_eq!(timers.epoch().1, 0);
            assert_eq!(timers.next(), Some(deadline));

            assert_eq!(timers.expire_timers(deadline), 1);
            assert!(wakers.is_awoken(n));

            // Should have advanced the epoch to come back around to 0.
            assert_eq!(timers.epoch().1, 0);
            assert_eq!(timers.expire_timers(epoch), 0);
        }

        #[test]
        fn add_deadline_to_all_slots() {
            let timers = SharedTimers::new();
            let mut wakers = WakerBuilder::<{ SLOTS + 1 }>::new();
            let epoch = timers.epoch().0;

            // Add a deadline to all slots and the overflow list.
            for n in 0..=SLOTS {
                let deadline = epoch + Duration::from_nanos((n as u64 * NS_PER_SLOT as u64) + 10);
                let (n2, waker) = wakers.task_waker();
                assert_eq!(n, n2);
                _ = timers.add(deadline, waker);
            }

            let first_deadline = epoch + Duration::from_nanos(10);
            assert_eq!(timers.next(), Some(first_deadline));
            assert_eq!(timers.expire_timers(epoch), 0);
            assert_eq!(timers.epoch().1, 0);

            let mut expected_next_deadline = first_deadline;
            let mut expected_index = 0;
            for n in 0..=SLOTS {
                assert_eq!(timers.next(), Some(expected_next_deadline));
                let now = expected_next_deadline + Duration::from_nanos(1);
                assert_eq!(timers.expire_timers(now), 1);
                assert!(wakers.is_awoken(n));
                assert_eq!(timers.epoch().1, expected_index);

                assert_eq!(timers.expire_timers(now), 0);
                assert_eq!(timers.epoch().1, expected_index);

                expected_index = (expected_index + 1) % SLOTS as u8;
                expected_next_deadline += DURATION_PER_SLOT;
            }
        }

        #[test]
        fn add_deadline_in_the_past() {
            let timers = SharedTimers::new();
            let mut wakers = WakerBuilder::<1>::new();
            let epoch = timers.epoch().0;

            let (n, waker) = wakers.task_waker();
            _ = timers.add(epoch - Duration::from_secs(1), waker);
            assert_eq!(timers.next(), Some(epoch));

            assert_eq!(timers.expire_timers(epoch), 1);
            assert!(wakers.is_awoken(n));
        }

        #[test]
        fn adding_earlier_deadline() {
            let timers = SharedTimers::new();
            let mut wakers = WakerBuilder::<2>::new();
            let epoch = timers.epoch().0;

            let (n1, waker) = wakers.task_waker();
            let deadline1 = epoch + Duration::from_secs(2);
            _ = timers.add(deadline1, waker);
            assert_eq!(timers.next(), Some(deadline1));

            let (n2, waker) = wakers.task_waker();
            let deadline2 = epoch + Duration::from_secs(1);
            _ = timers.add(deadline2, waker);
            assert_eq!(timers.next(), Some(deadline2));

            assert_eq!(timers.expire_timers(deadline1), 2);
            assert!(wakers.is_awoken(n1));
            assert!(wakers.is_awoken(n2));
            assert_eq!(timers.expire_timers(deadline1), 0);
        }

        #[test]
        fn remove_deadline() {
            let timers = SharedTimers::new();
            let mut wakers = WakerBuilder::<1>::new();
            let epoch = timers.epoch().0;

            let deadline = epoch + Duration::from_millis(10);
            let (_, waker) = wakers.task_waker();
            let token = timers.add(deadline, waker);
            assert_eq!(timers.next(), Some(deadline));

            timers.remove(deadline, token);
            assert_eq!(timers.next(), None);
            assert_eq!(timers.expire_timers(epoch), 0);
        }

        #[test]
        fn remove_never_added_deadline() {
            let timers = SharedTimers::new();
            let epoch = timers.epoch().0;

            assert_eq!(timers.next(), None);
            assert_eq!(timers.expire_timers(epoch), 0);
            let deadline = epoch + Duration::from_millis(10);
            timers.remove(deadline, TimerToken(0));
            assert_eq!(timers.next(), None);
            assert_eq!(timers.expire_timers(epoch), 0);
        }

        #[test]
        fn remove_expired_deadline() {
            let timers = SharedTimers::new();
            let mut wakers = WakerBuilder::<1>::new();
            let epoch = timers.epoch().0;

            let deadline = epoch + Duration::from_millis(10);
            let (n, waker) = wakers.task_waker();
            let token = timers.add(deadline, waker);
            assert_eq!(timers.next(), Some(deadline));

            assert_eq!(timers.expire_timers(deadline), 1);
            assert!(wakers.is_awoken(n));

            timers.remove(deadline, token);
            assert_eq!(timers.next(), None);
            assert_eq!(timers.expire_timers(epoch), 0);
        }

        #[test]
        fn remove_deadline_from_all_slots() {
            let timers = SharedTimers::new();
            let mut wakers = WakerBuilder::<{ SLOTS + 1 }>::new();
            let epoch = timers.epoch().0;

            // Add a deadline to all slots and the overflow list.
            let tokens: Vec<TimerToken> = (0..=SLOTS)
                .into_iter()
                .map(|n| {
                    let deadline =
                        epoch + Duration::from_nanos((n as u64 * NS_PER_SLOT as u64) + 10);
                    let (n2, waker) = wakers.task_waker();
                    assert_eq!(n, n2);
                    timers.add(deadline, waker)
                })
                .collect();

            let first_deadline = epoch + Duration::from_nanos(10);
            assert_eq!(timers.next(), Some(first_deadline));
            assert_eq!(timers.expire_timers(epoch), 0);
            assert_eq!(timers.epoch().1, 0);

            let mut next_deadline = first_deadline;
            for (n, token) in tokens.into_iter().enumerate() {
                timers.remove(next_deadline, token);
                next_deadline += DURATION_PER_SLOT;

                if n == SLOTS {
                    assert_eq!(timers.next(), None);
                } else {
                    assert_eq!(timers.next(), Some(next_deadline));
                }
            }
        }

        #[test]
        fn remove_deadline_from_all_slots_interleaved() {
            let timers = SharedTimers::new();
            let mut wakers = WakerBuilder::<{ SLOTS + 1 }>::new();
            let epoch = timers.epoch().0;

            // Add a deadline to all slots and the overflow list.
            for n in 0..=SLOTS {
                let deadline = epoch + Duration::from_nanos((n as u64 * NS_PER_SLOT as u64) + 10);
                let (n2, waker) = wakers.task_waker();
                assert_eq!(n, n2);
                let token = timers.add(deadline, waker);
                timers.remove(deadline, token);
            }

            assert_eq!(timers.next(), None);
            assert_eq!(timers.expire_timers(epoch), 0);
            assert_eq!(timers.epoch().1, 0);
        }

        #[test]
        fn remove_deadline_after_epoch_advance() {
            let timers = SharedTimers::new();
            let mut wakers = WakerBuilder::<{ SLOTS + 1 }>::new();
            let epoch = timers.epoch().0;

            // Add a deadline to all slots and the overflow list.
            let tokens: Vec<TimerToken> = (0..=SLOTS)
                .into_iter()
                .map(|n| {
                    let deadline =
                        epoch + Duration::from_nanos((n as u64 * NS_PER_SLOT as u64) + 10);
                    let (n2, waker) = wakers.task_waker();
                    assert_eq!(n, n2);
                    timers.add(deadline, waker)
                })
                .skip(1)
                .collect();

            let first_deadline = epoch + Duration::from_nanos(10);
            let now = epoch + DURATION_PER_SLOT;
            assert_eq!(timers.next(), Some(first_deadline));
            assert_eq!(timers.expire_timers(now), 1);
            assert!(wakers.is_awoken(0));
            assert_eq!(timers.epoch().1, 1);
            assert_eq!(timers.next(), Some(first_deadline + DURATION_PER_SLOT));

            let mut next_deadline = first_deadline + DURATION_PER_SLOT;
            for (n, token) in tokens.into_iter().enumerate() {
                timers.remove(next_deadline, token);
                next_deadline += DURATION_PER_SLOT;

                if n == SLOTS - 1 {
                    assert_eq!(timers.next(), None);
                } else {
                    assert_eq!(timers.next(), Some(next_deadline));
                }
            }
        }

        #[test]
        fn remove_deadline_in_the_past() {
            let timers = SharedTimers::new();
            let mut wakers = WakerBuilder::<1>::new();
            let epoch = timers.epoch().0;

            let deadline = epoch - Duration::from_secs(1);
            let (_, waker) = wakers.task_waker();
            let token = timers.add(deadline, waker);
            assert_eq!(timers.next(), Some(epoch));

            timers.remove(deadline, token);
            assert_eq!(timers.next(), None);
            assert_eq!(timers.expire_timers(epoch), 0);
        }
    }
}
