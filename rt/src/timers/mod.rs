//! Timers implementation.
//!
//! This module hold the timer**s** implementation, that is the collection of
//! timers currently in the runtime. Also see the [`timer`] implementation,
//! which exposes types to the user.
//!
//! [`timer`]: crate::timer
//!
//! # Implementation
//!
//! This implementation is based on a Timing Wheel as discussed in the paper
//! "Hashed and hierarchical timing wheels: efficient data structures for
//! implementing a timer facility" by George Varghese and Anthony Lauck (1997).
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

pub(crate) mod shared;
#[cfg(test)]
mod tests;

pub(crate) use crate::rt::TimerToken;

use std::task;
use std::time::{Duration, Instant};

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

/// A timer in [`Timers`].
#[derive(Debug)]
struct Timer<T> {
    deadline: T,
    waker: task::Waker,
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
    let token = TimerToken::new(waker.data() as usize);
    timers.insert(idx, Timer { deadline, waker });
    token
}

/// Remove a previously added `deadline` from `timers`, ensuring it remains sorted.
#[allow(clippy::needless_pass_by_value)]
fn remove_timer<T: Ord>(timers: &mut Vec<Timer<T>>, deadline: T, token: TimerToken) {
    if let Ok(idx) = timers.binary_search_by(|timer| timer.deadline.cmp(&deadline)) {
        if timers[idx].waker.data() as usize == token.data() {
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
