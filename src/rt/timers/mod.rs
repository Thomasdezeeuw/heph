// TODO:
// * Benchmark different `WHEEL_SIZE`s (cpu and mem).

use std::cmp::Reverse;
use std::iter::FusedIterator;
use std::time::{Duration, Instant};

use crate::rt::ProcessId;

#[cfg(test)]
mod tests;

/// Number of buckets in the wheel.
///
/// Keep this a power of 2 for magic improved performance.
const WHEEL_SIZE: usize = 32;

/// Time unit used as deadline.
///
/// This represents the number of milliseconds since the epoch of `TimingWheel`.
///
/// This gives roughly `(2 ^ 64) / 1000 / 60 / 60 / 24 / 365` ~= 584,942,417
/// years of space since the `TimingWheel::epoch`.
type Time = u64;

/// Collection of timers.
///
/// Loosely based on the paper ``Hashed and hierarchical timing wheels:
/// efficient data structures for implementing a timer facility'' by George
/// Varghese and Anthony Lauck (1997). We're using Scheme 5 - Hash Table With
/// Sorted Lists.
#[derive(Debug)]
pub(crate) struct TimingWheel {
    buckets: [Bucket; WHEEL_SIZE],
    /// The "zero" time, all deadlines (of type `Time`) are based on this.
    epoch: Instant,
}

impl TimingWheel {
    /// Create a new `TimingWheel`.
    pub(crate) fn new() -> TimingWheel {
        TimingWheel::setup(Instant::now())
    }

    const fn setup(epoch: Instant) -> TimingWheel {
        TimingWheel {
            buckets: [Bucket::EMPTY; WHEEL_SIZE],
            epoch,
        }
    }

    /// Add a new deadline.
    ///
    /// # Panics
    ///
    /// Panics if `deadline` is in the past.
    #[track_caller]
    pub(super) fn add_deadline(&mut self, pid: ProcessId, deadline: Instant) {
        let deadline = self.elapsed_since_epoch(deadline);
        let timer = Timer { pid, deadline };
        self.bucket_for(timer).add(timer);
    }

    /// Remove a previously added deadline.
    ///
    /// # Notes
    ///
    /// Does nothing if the deadline was not found, e.g. when it has expired or
    /// was never added in the first place.
    #[track_caller]
    pub(super) fn remove_deadline(&mut self, pid: ProcessId, deadline: Instant) {
        let deadline = self.elapsed_since_epoch(deadline);
        let timer = Timer { pid, deadline };
        self.bucket_for(timer).remove(timer);
    }

    /// Returns the [`Bucker`] in which to place `timer`, or in which `timer`
    /// should be.
    fn bucket_for(&mut self, timer: Timer) -> &mut Bucket {
        &mut self.buckets[timer.deadline as usize % self.buckets.len()]
    }

    /// Returns the time of the next deadline, if any.
    pub(super) fn next_deadline(&self) -> Option<Instant> {
        let mut next_deadline = None;
        for bucket in self.buckets.iter() {
            if let Some(time) = bucket.next() {
                match &mut next_deadline {
                    next @ None => *next = Some(time),
                    Some(next_deadline) if time < *next_deadline => {
                        *next_deadline = time;
                    }
                    _ => { /* `next_deadline` is earlier. */ }
                }
            }
        }

        next_deadline.map(|next| self.epoch + Duration::from_millis(next))
    }

    /// Returns an iterator that iterates over all deadlines expired since
    /// `now`.
    #[track_caller]
    pub(super) fn deadlines(&mut self, now: Instant) -> Deadlines<'_> {
        let elapsed = self.elapsed_since_epoch(now);
        Deadlines {
            wheel: self,
            elapsed,
            current_bucket: 0,
            read_from_bucket: 0,
        }
    }

    /// Returns the number of milliseconds elapsed since the current epoch,
    /// based on the current time as defined in `time`.
    #[track_caller]
    fn elapsed_since_epoch(&self, time: Instant) -> Time {
        time.duration_since(self.epoch).as_millis() as Time
    }
}

/// A collection of `Timer`s of a `TimingWheel`.
#[derive(Debug)]
struct Bucket {
    /// This vectored is sorted most of the time, When a timer is removed it's
    /// actually not removed but it's `pid` is set to invalid. See
    /// [`Bucket::add`] and [`Bucket::remove`].
    timers: Vec<Reverse<Timer>>,
}

impl Bucket {
    /// Empty `Bucket`.
    const EMPTY: Bucket = Bucket { timers: Vec::new() };

    /// Returns the next `Time` to expire, if any.
    fn next(&self) -> Option<Time> {
        self.timers
            .iter()
            .rev()
            .find_map(|timer| timer.0.pid.is_valid().then(|| timer.0.deadline))
    }

    /// Add `timer` to the bucket.
    fn add(&mut self, timer: Timer) {
        let res = self
            .timers
            .binary_search_by(|t| timer.deadline.cmp(&t.0.deadline));
        let idx = match res {
            Ok(idx) => idx,
            Err(idx) => idx,
        };
        self.timers.insert(idx, Reverse(timer));
    }

    /// Remove `timer` from the bucket.
    fn remove(&mut self, timer: Timer) {
        let idx = self.timers.binary_search_by(|t| {
            timer
                .deadline
                .cmp(&t.0.deadline)
                .then_with(|| timer.pid.cmp(&t.0.pid))
        });
        if let Ok(idx) = idx {
            // We replace the pid with an invalid one so we don't trigger the
            // timer, but also don't have to potentially move a bunch of timers
            // around to the remove the data from the list.
            self.timers[idx].0.pid.invalidate();
        }
    }

    /// Returns the timers in order of earlier expiration.
    fn timers(&self) -> impl Iterator<Item = &Reverse<Timer>> {
        self.timers.iter().rev()
    }
}

/// Representation of a timer in the `TimingWheel`.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct Timer {
    /// Process that created the timer.
    pid: ProcessId,
    /// The time at which the timer expires.
    deadline: Time,
}

impl Timer {
    /// Returns `true` if the timer has elapsed, using `now` as the current
    /// time.
    const fn is_elapsed(&self, now: Time) -> bool {
        self.deadline <= now
    }
}

/// Iterator of expired deadlines, see [`TimingWheel::deadlines`].
pub(super) struct Deadlines<'a> {
    wheel: &'a mut TimingWheel,
    /// Elapsed milliseconds since the current epoch.
    elapsed: Time,
    /// Index of the current bucket (`self.wheel.buckets[self.current_backet]`).
    current_bucket: usize,
    /// Number of timers read (removed) from the current bucket.
    read_from_bucket: usize,
}

impl<'a> Iterator for Deadlines<'a> {
    type Item = ProcessId;

    fn next(&mut self) -> Option<Self::Item> {
        for bucket in self.wheel.buckets.iter_mut().skip(self.current_bucket) {
            for timer in bucket.timers().skip(self.read_from_bucket) {
                if timer.0.is_elapsed(self.elapsed) {
                    self.read_from_bucket += 1;
                    if timer.0.pid.is_valid() {
                        // NOTE: we don't remove the timer from the bucket yet,
                        // that happens after we removed all expired timers from
                        // this bucket (or in the `Drop` impl if we get dropped
                        // before we reach that code).
                        return Some(timer.0.pid);
                    }
                } else {
                    // No more expired timers in this bucket.
                    break;
                }
            }

            // Remove all the timers from the bucket that we've returned
            // already.
            bucket
                .timers
                .truncate(bucket.timers.len() - self.read_from_bucket);

            // Move to the next bucket.
            self.current_bucket += 1;
            self.read_from_bucket = 0;
        }

        None
    }
}

impl<'a> FusedIterator for Deadlines<'a> {}

impl<'a> Drop for Deadlines<'a> {
    fn drop(&mut self) {
        if let Some(bucket) = self.wheel.buckets.get_mut(self.current_bucket) {
            // We're getting dropped part way through iterating, ensure that all
            // timers that we returned (while iterating) are removed from the
            // bucket.
            bucket
                .timers
                .truncate(bucket.timers.len() - self.read_from_bucket);
        }
    }
}
