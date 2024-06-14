//! Bounded capacity channel.
//!
//! The channel is a multi-producer, single-consumer (MPSC) bounded queue. It is
//! designed to be used as inbox for actors, following the [actor model].
//!
//! [actor model]: https://en.wikipedia.org/wiki/Actor_model
//!
//! # Notes
//!
//! The implementation assumes the access to the channel is mostly uncontested
//! and optimises for this use case. Furthermore it optimises for small memory
//! footprint, sometimes over faster access.
//!
//! The implementation doesn't provide a lot of guarantees. For example this
//! channel is **not** guaranteed to be First In First Out (FIFO), it does this
//! on a best effort basis. In return it means that a slow `Sender` does not
//! block the receiving of other messages.
//!
//! # Examples
//!
//! Simple creation of a channel and sending a message over it.
//!
//!```
//! use std::thread;
//!
//! use heph_inbox::RecvError;
//!
//! // Create a new small channel.
//! let (sender, mut receiver) = heph_inbox::new_small();
//!
//! let sender_handle = thread::spawn(move || {
//!     if let Err(err) = sender.try_send("Hello world!".to_owned()) {
//!         panic!("Failed to send value: {err}");
//!     }
//! });
//!
//! let receiver_handle = thread::spawn(move || {
//! #   #[cfg(not(miri))] // `sleep` not supported.
//! #   thread::sleep(std::time::Duration::from_millis(1)); // Don't waste cycles.
//!     // NOTE: this is just an example don't actually use a loop like this, it
//!     // will waste CPU cycles when the channel is empty!
//!     loop {
//!         match receiver.try_recv() {
//!             Ok(value) => println!("Got a value: {value}"),
//!             Err(RecvError::Empty) => continue,
//!             Err(RecvError::Disconnected) => break,
//!         }
//!     }
//! });
//!
//! sender_handle.join().unwrap();
//! receiver_handle.join().unwrap();
//! ```

#![feature(cfg_sanitize)]
#![warn(
    missing_debug_implementations,
    missing_docs,
    unused_results,
    variant_size_differences
)]
// Disallow warnings when running tests.
#![cfg_attr(test, deny(warnings))]
// Disallow warnings in examples, we want to set a good example after all.
#![doc(test(attr(deny(warnings))))]

use std::alloc::{alloc, handle_alloc_error, Layout};
use std::cell::UnsafeCell;
use std::error::Error;
use std::fmt;
use std::future::Future;
use std::mem::{drop as unlock, replace, take, MaybeUninit};
use std::ops::Deref;
use std::panic::{RefUnwindSafe, UnwindSafe};
use std::pin::Pin;
use std::ptr::{self, NonNull};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::task::{self, Poll};

#[cfg(test)]
mod tests;

/// `ThreadSanitizer` does not support memory fences. To avoid false positive
/// reports use atomic loads for synchronization instead of a fence. Macro
/// inspired by the one found in Rust's standard library for the `Arc`
/// implementation.
macro_rules! fence {
    ($val: expr, $ordering: expr) => {
        #[cfg(not(sanitize = "thread"))]
        std::sync::atomic::fence($ordering);
        #[cfg(sanitize = "thread")]
        {
            _ = $val.load($ordering);
        }
    };
}

pub mod oneshot;

mod waker;
use waker::WakerRegistration;

/// The capacity of a small channel.
const SMALL_CAP: usize = 8;
/// Maximum capacity of a channel.
// NOTE: see [`Channel::new`] why.
pub const MAX_CAP: usize = 29;
/// Minimum capacity of a channel.
pub const MIN_CAP: usize = 1;

/// Create a small bounded channel.
pub fn new_small<T>() -> (Sender<T>, Receiver<T>) {
    new(SMALL_CAP)
}

/// Create a new bounded channel.
///
/// The `capacity` must be in the range [`MIN_CAP`]`..=`[`MAX_CAP`].
pub fn new<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    assert!(
        (MIN_CAP..=MAX_CAP).contains(&capacity),
        "inbox channel capacity must be between {MIN_CAP} and {MAX_CAP}",
    );
    let channel = Channel::new(capacity);
    let sender = Sender { channel };
    let receiver = Receiver { channel };
    (sender, receiver)
}

/// Bit mask to mark the receiver as alive.
const RECEIVER_ALIVE: usize = 1 << (usize::BITS - 1);
/// Bit mask to mark the receiver still has access to the channel. See the
/// `Drop` impl for [`Receiver`].
const RECEIVER_ACCESS: usize = 1 << (usize::BITS - 2);
/// Bit mask to mark a sender still has access to the channel. See the `Drop`
/// impl for [`Sender`].
const SENDER_ACCESS: usize = 1 << (usize::BITS - 3);
/// Bit mask to mark the manager as alive.
const MANAGER_ALIVE: usize = 1 << (usize::BITS - 4);
/// Bit mask to mark the manager has access to the channel. See the `Drop` impl
/// for [`Manager`].
const MANAGER_ACCESS: usize = 1 << (usize::BITS - 5);

/// Return `true` if the receiver or manager is alive in `ref_count`.
const fn has_receiver(ref_count: usize) -> bool {
    ref_count & RECEIVER_ALIVE != 0
}

/// Returns `true` if the manager is alive in `ref_count`.
const fn has_manager(ref_count: usize) -> bool {
    ref_count & MANAGER_ALIVE != 0
}

/// Return `true` if the receiver or manager is alive in `ref_count`.
const fn has_receiver_or_manager(ref_count: usize) -> bool {
    ref_count & (RECEIVER_ALIVE | MANAGER_ALIVE) != 0
}

/// Returns the number of senders connected in `ref_count`.
const fn sender_count(ref_count: usize) -> usize {
    ref_count & !(RECEIVER_ALIVE | RECEIVER_ACCESS | SENDER_ACCESS | MANAGER_ALIVE | MANAGER_ACCESS)
}

// Bits to mark the status of a slot.
const STATUS_BITS: u64 = 2; // Number of bits used per slot.
const STATUS_MASK: u64 = (1 << STATUS_BITS) - 1;
#[cfg(test)]
const ALL_STATUSES_MASK: u64 = (1 << (MAX_CAP as u64 * STATUS_BITS)) - 1;
// The possible statuses of a slot.
const EMPTY: u64 = 0b00; // Slot is empty (initial state).
const TAKEN: u64 = 0b01; // `Sender` acquired write access, currently writing.
const FILLED: u64 = 0b11; // `Sender` wrote a value into the slot.
const READING: u64 = 0b10; // A `Receiver` is reading from the slot.

// Status transitions.
const MARK_TAKEN: u64 = 0b01; // OR to go from EMPTY -> TAKEN.
const MARK_FILLED: u64 = 0b11; // OR to go from TAKEN -> FILLED.
const MARK_READING: u64 = 0b01; // XOR to go from FILLED -> READING.
const MARK_EMPTIED: u64 = 0b11; // ! AND to go from FILLED or READING -> EMPTY.

/// Returns `true` if `slot` in `status` is empty.
const fn is_available(status: u64, slot: usize) -> bool {
    has_status(status, slot, EMPTY)
}

/// Returns `true` if `slot` in `status` is filled.
const fn is_filled(status: u64, slot: usize) -> bool {
    has_status(status, slot, FILLED)
}

/// Returns `true` if `slot` (in `status`) equals the `expected` status.
const fn has_status(status: u64, slot: usize, expected: u64) -> bool {
    slot_status(status, slot) == expected
}

/// Returns the `STATUS_BITS` for `slot` in `status`.
const fn slot_status(status: u64, slot: usize) -> u64 {
    debug_assert!(slot <= MAX_CAP);
    (status >> (STATUS_BITS * slot as u64)) & STATUS_MASK
}

/// Creates a mask to transition `slot` using `transition`. `transition` must be
/// one of the `MARK_*` constants.
const fn mark_slot(slot: usize, transition: u64) -> u64 {
    debug_assert!(slot <= MAX_CAP);
    transition << (STATUS_BITS * slot as u64)
}

/// Returns a string name for the `slot_status`.
const fn dbg_status(slot_status: u64) -> &'static str {
    match slot_status {
        EMPTY => "EMPTY",
        TAKEN => "TAKEN",
        FILLED => "FILLED",
        READING => "READING",
        _ => "INVALID",
    }
}

// Bits to mark the position of the receiver.
const MARK_NEXT_POS: u64 = 1 << (STATUS_BITS * MAX_CAP as u64); // Add to increase position by 1.

/// Returns the position of the receiver. Will be in 0..[`MAX_CAP`] range.
#[allow(clippy::cast_possible_truncation)]
const fn receiver_pos(status: u64, capacity: usize) -> usize {
    (status >> (STATUS_BITS * MAX_CAP as u64)) as usize % capacity
}

/// Sending side of the channel.
pub struct Sender<T> {
    channel: NonNull<Channel<T>>,
}

/// Error returned in case sending a value across the channel fails. See
/// [`Sender::try_send`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SendError<T> {
    /// Channel is full.
    Full(T),
    /// [`Receiver`] and [`Manager`] are disconnected.
    Disconnected(T),
}

impl<T> fmt::Display for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SendError::Full(..) => f.pad("channel is full"),
            SendError::Disconnected(..) => f.pad("receiver is disconnected"),
        }
    }
}

impl<T: fmt::Debug> Error for SendError<T> {}

impl<T> Sender<T> {
    /// Attempts to send the `value` into the channel.
    pub fn try_send(&self, value: T) -> Result<(), SendError<T>> {
        try_send(self.channel(), value)
    }

    /// Returns a future that sends a value into the channel, waiting if the
    /// channel is full.
    ///
    /// If the returned [`Future`] returns an error it means the [`Receiver`]
    /// and [`Manager`] are [disconnected] and no more values will be read from
    /// the channel. This is the same error as [`SendError::Disconnected`].
    /// [`SendError::Full`] will never be returned, the `Future` will return
    /// [`Poll::Pending`] instead.
    ///
    /// [disconnected]: Sender::is_connected
    pub fn send(&self, value: T) -> SendValue<T> {
        SendValue {
            channel: self.channel(),
            value: Some(value),
            registered_waker: None,
        }
    }

    /// Returns a [`Future`] that waits until the other side of the channel is
    /// [disconnected].
    ///
    /// [disconnected]: Sender::is_connected
    pub fn join(&self) -> Join<T> {
        Join {
            channel: self.channel(),
            registered_waker: None,
        }
    }

    /// Returns the capacity of the channel.
    pub fn capacity(&self) -> usize {
        self.channel().slots.len()
    }

    /// Returns `true` if the [`Receiver`] and or the [`Manager`] are connected.
    ///
    /// # Notes
    ///
    /// Unlike [`Receiver::is_connected`] this method takes the [`Manager`] into
    /// account. This is done to support the use case in which an actor is
    /// restarted and a new receiver is created for it.
    pub fn is_connected(&self) -> bool {
        // Relaxed is fine here since there is always a bit of a race condition
        // when using this method (and then doing something based on it).
        has_receiver_or_manager(self.channel().ref_count.load(Ordering::Relaxed))
    }

    /// Returns `true` if the [`Manager`] is connected.
    pub fn has_manager(&self) -> bool {
        // Relaxed is fine here since there is always a bit of a race condition
        // when using this method (and then doing something based on it).
        has_manager(self.channel().ref_count.load(Ordering::Relaxed))
    }

    /// Returns `true` if senders send into the same channel.
    pub fn same_channel(&self, other: &Sender<T>) -> bool {
        ptr::addr_eq(self.channel.as_ptr(), other.channel.as_ptr())
    }

    /// Returns `true` if this sender sends to the `receiver`.
    pub fn sends_to(&self, receiver: &Receiver<T>) -> bool {
        ptr::addr_eq(self.channel.as_ptr(), receiver.channel.as_ptr())
    }

    /// Returns the id of this sender.
    pub fn id(&self) -> Id {
        Id(self.channel.as_ptr().cast_const().cast::<()>() as usize)
    }

    fn channel(&self) -> &Channel<T> {
        unsafe { self.channel.as_ref() }
    }
}

/// See [`Sender::try_send`].
fn try_send<T>(channel: &Channel<T>, value: T) -> Result<(), SendError<T>> {
    if !has_receiver_or_manager(channel.ref_count.load(Ordering::Relaxed)) {
        return Err(SendError::Disconnected(value));
    }

    // NOTE: relaxed ordering here is ok because we acquire unique
    // permission to write to the slot later before writing to it. Something
    // we have to do no matter the ordering.
    let mut status: u64 = channel.status.load(Ordering::Relaxed);
    let cap = channel.slots.len();
    let start = receiver_pos(status, cap);
    for slot in (0..cap).cycle().skip(start).take(cap) {
        if !is_available(status, slot) {
            continue;
        }

        // In our local status the slot is available, however another sender
        // could have taken it between the time we read the status and the
        // time we got here. So we write our `TAKEN` status and check if in
        // the *previous* (up-to-date) status (returned by `fetch_or`) the
        // slot was still available. If it was it means we have acquired the
        // slot, otherwise another sender beat us to it.
        //
        // NOTE: The OR operation here is safe: if another sender already
        // wrote TAKEN (01) or FILLED (11) we're not overwriting anything.
        // If a reader wrote READING (10) we won't use the slot and the
        // reader will overwrite it with EMPTY later. If we overwrite EMPTY
        // (00) we can reuse the slot safely, but the message will be in a
        // different order.
        status = channel
            .status
            .fetch_or(mark_slot(slot, MARK_TAKEN), Ordering::AcqRel);
        if !is_available(status, slot) {
            // Another thread beat us to taking the slot.
            continue;
        }

        // SAFETY: we've acquired the slot above so we're ensured unique
        // access to the slot.
        unsafe {
            let _: &mut T = (*channel.slots[slot].get()).write(value);
        }

        // Now we've writing to the slot we can mark it slot as filled.
        let old_status = channel
            .status
            .fetch_or(mark_slot(slot, MARK_FILLED), Ordering::AcqRel);
        // Debug assertion to check the slot was in the TAKEN status.
        debug_assert!(has_status(old_status, slot, TAKEN));

        // If the receiver is waiting for this lot we wake it.
        if receiver_pos(old_status, cap) == slot {
            channel.wake_receiver();
        }

        return Ok(());
    }

    Err(SendError::Full(value))
}

/// # Safety
///
/// Only `2 ^ 30` (a billion) `Sender`s may be alive concurrently, more than
/// enough for most practical use cases.
impl<T> Clone for Sender<T> {
    fn clone(&self) -> Sender<T> {
        // SAFETY: for the reasoning behind this relaxed ordering see `Arc::clone`.
        let old_ref_count = self.channel().ref_count.fetch_add(1, Ordering::Relaxed);
        debug_assert!(old_ref_count & SENDER_ACCESS != 0);
        Sender {
            channel: self.channel,
        }
    }
}

impl<T> fmt::Debug for Sender<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Sender")
            .field("channel", &self.channel())
            .finish()
    }
}

impl<T> Unpin for Sender<T> {}

// SAFETY: if the value can be send across thread than so can the channel.
unsafe impl<T: Send> Send for Sender<T> {}
unsafe impl<T> Sync for Sender<T> {}

impl<T: RefUnwindSafe> RefUnwindSafe for Sender<T> {}
impl<T: RefUnwindSafe> UnwindSafe for Sender<T> {}

impl<T> Drop for Sender<T> {
    #[rustfmt::skip]
    fn drop(&mut self) {
        // SAFETY: for the reasoning behind this ordering see `Arc::drop`.
        let old_ref_count = self.channel().ref_count.fetch_sub(1, Ordering::Release);
        if sender_count(old_ref_count) != 1 {
            // If we're not the last sender all we have to do is decrement the
            // ref count (above).
            return;
        }

        // If we're the last sender being dropped wake the receiver.
        if has_receiver_or_manager(old_ref_count) {
            self.channel().wake_receiver();
        }

        // If the previous value was `SENDER_ACCESS` it means that the receiver,
        // all other senders and the manager were all dropped, so we need to do
        // the deallocating.
        let old_ref_count = self.channel().ref_count.fetch_and(!SENDER_ACCESS, Ordering::Release);
        if old_ref_count != SENDER_ACCESS {
            // Another sender, the receiver or the manager is still alive.
            return;
        }

        // SAFETY: for the reasoning behind this ordering see `Arc::drop`.
        fence!(self.channel().ref_count, Ordering::Acquire);

        // Drop the memory.
        unsafe { drop(Box::from_raw(self.channel.as_ptr())) }
    }
}

/// [`Future`] implementation behind [`Sender::send`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct SendValue<'s, T> {
    channel: &'s Channel<T>,
    value: Option<T>,
    registered_waker: Option<task::Waker>,
}

impl<'s, T> Future for SendValue<'s, T> {
    type Output = Result<(), T>;

    fn poll(mut self: Pin<&mut Self>, ctx: &mut task::Context) -> Poll<Self::Output> {
        // SAFETY: only `waker_node` is pinned, which is only used by
        // `register_waker`.
        let this = unsafe { self.as_mut().get_unchecked_mut() };
        let value = this
            .value
            .take()
            .expect("SendValue polled after completion");

        // First we try to send the value, if this succeeds we don't have to
        // allocate in the waker list.
        match try_send(this.channel, value) {
            Ok(()) => Poll::Ready(Ok(())),
            Err(SendError::Full(value)) => {
                let registered_waker = register_waker(
                    &mut this.registered_waker,
                    &this.channel.sender_wakers,
                    ctx.waker(),
                );
                if !registered_waker {
                    return Poll::Pending;
                }

                // It could be the case that the received received a value in
                // the time after we tried to send the value and before we added
                // the our waker to list. So we try to send a value again to
                // ensure we don't awoken and the channel has a slot available.
                match try_send(this.channel, value) {
                    Ok(()) => Poll::Ready(Ok(())),
                    Err(SendError::Full(value)) => {
                        // Channel is still full, we'll have to wait.
                        this.value = Some(value);
                        Poll::Pending
                    }
                    Err(SendError::Disconnected(value)) => Poll::Ready(Err(value)),
                }
            }
            Err(SendError::Disconnected(value)) => Poll::Ready(Err(value)),
        }
    }
}

unsafe impl<'s, T> Sync for SendValue<'s, T> {}

impl<'s, T> Drop for SendValue<'s, T> {
    fn drop(&mut self) {
        // If we registered a waker remove ourselves from the list.
        if let Some(waker) = self.registered_waker.take() {
            let mut sender_wakers = self.channel.sender_wakers.lock().unwrap();
            let idx = sender_wakers.iter().position(|w| w.will_wake(&waker));
            if let Some(idx) = idx {
                let waker = sender_wakers.swap_remove(idx);
                unlock(sender_wakers);
                drop(waker);
            }
        }
    }
}

/// [`Future`] implementation behind [`Sender::join`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct Join<'s, T> {
    channel: &'s Channel<T>,
    registered_waker: Option<task::Waker>,
}

impl<'s, T> Future for Join<'s, T> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, ctx: &mut task::Context) -> Poll<Self::Output> {
        if !has_receiver_or_manager(self.channel.ref_count.load(Ordering::Acquire)) {
            // Other side is disconnected.
            return Poll::Ready(());
        }

        let this = &mut *self;
        let registered_waker = &mut this.registered_waker;
        let join_wakers = &this.channel.join_wakers;
        let registered_waker = register_waker(registered_waker, join_wakers, ctx.waker());
        if !registered_waker {
            return Poll::Pending;
        }

        if has_receiver_or_manager(this.channel.ref_count.load(Ordering::Acquire)) {
            Poll::Pending
        } else {
            // Other side is disconnected.
            Poll::Ready(())
        }
    }
}

unsafe impl<'s, T> Sync for Join<'s, T> {}

impl<'s, T> Drop for Join<'s, T> {
    fn drop(&mut self) {
        if let Some(waker) = self.registered_waker.take() {
            let mut join_wakers = self.channel.join_wakers.lock().unwrap();
            let idx = join_wakers.iter().position(|w| w.will_wake(&waker));
            if let Some(idx) = idx {
                let waker = join_wakers.swap_remove(idx);
                unlock(join_wakers);
                drop(waker);
            }
        }
    }
}

/// Registers `waker` in `channel_wakers` if `registered_waker` is `None` or is
/// different from `waker`. Return `true` if `waker` was registered, `false`
/// otherwise.
fn register_waker(
    registered_waker: &mut Option<task::Waker>,
    channel_wakers: &Mutex<Vec<task::Waker>>,
    waker: &task::Waker,
) -> bool {
    match registered_waker {
        // Already registered this waker, don't have to do anything.
        Some(w) if w.will_wake(waker) => false,
        // Different waker, replace the old one.
        Some(w) => {
            let waker = waker.clone();
            let old_waker = replace(w, waker.clone());

            let mut channel_wakers = channel_wakers.lock().unwrap();
            let idx = channel_wakers.iter().position(|w| w.will_wake(&old_waker));
            if let Some(idx) = idx {
                // Replace the old waker with the new one.
                channel_wakers[idx] = waker;
            } else {
                // This can happen if `Sender` (or `Manager`) is being
                // dropped, most likely this `push` is pointless and we
                // return `Poll::Ready` below, but just in case.
                channel_wakers.push(waker);
            }
            true
        }
        // Haven't registered waker yet.
        None => {
            let waker = waker.clone();
            *registered_waker = Some(waker.clone());

            channel_wakers.lock().unwrap().push(waker);
            true
        }
    }
}

/// Receiving side of the channel.
pub struct Receiver<T> {
    channel: NonNull<Channel<T>>,
}

/// Error returned in case receiving a value from the channel fails. See
/// [`Receiver::try_recv`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum RecvError {
    /// Channel is empty.
    Empty,
    /// All [`Sender`]s (but not necessarily the [`Manager`]) are disconnected
    /// and the channel is empty, see [`Receiver::is_connected`].
    Disconnected,
}

impl fmt::Display for RecvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecvError::Empty => f.pad("channel is empty"),
            RecvError::Disconnected => f.pad("all senders are disconnected"),
        }
    }
}

impl Error for RecvError {}

impl<T> Receiver<T> {
    /// Attempts to receive a value from this channel.
    #[allow(clippy::needless_pass_by_ref_mut)]
    pub fn try_recv(&mut self) -> Result<T, RecvError> {
        try_recv(self.channel())
    }

    /// Returns a future that receives a value from the channel, waiting if the
    /// channel is empty.
    ///
    /// If the returned [`Future`] returns `None` it means all [`Sender`]s are
    /// [disconnected]. This is the same error as [`RecvError::Disconnected`].
    /// [`RecvError::Empty`] will never be returned, the `Future` will return
    /// [`Poll::Pending`] instead.
    ///
    /// [disconnected]: Receiver::is_connected
    #[allow(clippy::needless_pass_by_ref_mut)]
    pub fn recv(&mut self) -> RecvValue<T> {
        RecvValue {
            channel: self.channel(),
        }
    }

    /// Attempts to peek a value from this channel.
    #[allow(clippy::needless_pass_by_ref_mut)]
    pub fn try_peek(&mut self) -> Result<&T, RecvError> {
        try_peek(self.channel())
    }

    /// Returns a future that peeks at a value from the channel, waiting if the
    /// channel is empty.
    ///
    /// If the returned [`Future`] returns `None` it means all [`Sender`]s are
    /// [disconnected]. This is the same error as [`RecvError::Disconnected`].
    /// [`RecvError::Empty`] will never be returned, the `Future` will return
    /// [`Poll::Pending`] instead.
    ///
    /// [disconnected]: Receiver::is_connected
    #[allow(clippy::needless_pass_by_ref_mut)]
    pub fn peek(&mut self) -> PeekValue<T> {
        PeekValue {
            channel: self.channel(),
        }
    }

    /// Create a new [`Sender`] that sends to this channel.
    ///
    /// # Safety
    ///
    /// The same restrictions apply to this function as they do to
    /// [`Sender::clone`].
    ///
    /// [`Sender::clone`]: struct.Sender.html#impl-Clone-for-Sender<T>
    pub fn new_sender(&self) -> Sender<T> {
        // For the reasoning behind this relaxed ordering see `Arc::clone`.
        let old_ref_count = self.channel().ref_count.fetch_add(1, Ordering::Relaxed);
        if old_ref_count & SENDER_ACCESS != 0 {
            _ = self
                .channel()
                .ref_count
                .fetch_or(SENDER_ACCESS, Ordering::Relaxed);
        }

        Sender {
            channel: self.channel,
        }
    }

    /// Returns the capacity of the channel.
    pub fn capacity(&self) -> usize {
        self.channel().slots.len()
    }

    /// Returns `false` if all [`Sender`]s are disconnected.
    ///
    /// # Notes
    ///
    /// Unlike [`Sender::is_connected`] this method doesn't take the [`Manager`]
    /// into account. This means that this method can return `false` and later
    /// `true` (if the `Manager` created another `Sender`), which might be
    /// unexpected.
    pub fn is_connected(&self) -> bool {
        // Relaxed is fine here since there is always a bit of a race condition
        // when using this method (and then doing something based on it).
        sender_count(self.channel().ref_count.load(Ordering::Relaxed)) > 0
    }

    /// Returns `true` if the [`Manager`] is connected.
    pub fn has_manager(&self) -> bool {
        // Relaxed is fine here since there is always a bit of a race condition
        // when using this method (and then doing something based on it).
        has_manager(self.channel().ref_count.load(Ordering::Relaxed))
    }

    /// Set the receiver's waker to `waker`, if they are different. Returns
    /// `true` if the waker is changed, `false` otherwise.
    ///
    /// This is useful if you can't call [`Receiver::recv`] but still want a
    /// wake-up notification once messages are added to the inbox.
    #[allow(clippy::needless_pass_by_ref_mut)]
    pub fn register_waker(&mut self, waker: &task::Waker) -> bool {
        self.channel().receiver_waker.register(waker)
    }

    /// Returns the id of this receiver.
    pub fn id(&self) -> Id {
        Id(self.channel.as_ptr().cast_const().cast::<()>() as usize)
    }

    fn channel(&self) -> &Channel<T> {
        unsafe { self.channel.as_ref() }
    }
}

/// See [`Receiver::try_recv`].
fn try_recv<T>(channel: &Channel<T>) -> Result<T, RecvError> {
    // We check if we are connected **before** checking for messages. This
    // is important because there is a time between 1) the checking of the
    // messages in the channel and 2) checking if we're connected (if we
    // would do it in the last `if` statement of this method) in which the
    // sender could send a message and be dropped.
    // In this case, if we would check if we're connected after checking for
    // messages, we would incorrectly return `RecvError::Disconnected` (all
    // senders are dropped after all), however we would miss the last
    // message send.
    // Checking before hand causes us to return `RecvError::Empty`, which
    // technically isn't correct either but it will cause the user to check
    // again later. In `RecvValue` this is solved by calling `try_recv`
    // after registering the task waker, ensuring no wake-up events are
    // missed.
    let is_connected = sender_count(channel.ref_count.load(Ordering::Relaxed)) > 0;

    // Since we subtract from the `status` this will overflow at some point. But
    // `fetch_add` wraps-around on overflow, so the position will "reset" itself
    // to 0. This is one of the reasons we don't support FIFO order. The status
    // bits will not be touched (even on wrap-around).
    let mut status = channel.status.fetch_add(MARK_NEXT_POS, Ordering::AcqRel);
    let cap = channel.slots.len();
    let start = receiver_pos(status, cap);
    for slot in (0..cap).cycle().skip(start).take(cap) {
        if !is_filled(status, slot) {
            continue;
        }

        // Mark the slot as being read.
        status = channel
            .status
            .fetch_xor(mark_slot(slot, MARK_READING), Ordering::AcqRel);
        if !is_filled(status, slot) {
            // Slot isn't available after all.
            continue;
        }

        // SAFETY: we've acquired unique access to the slot above and we're
        // ensured the slot is filled.
        let value = unsafe { (*channel.slots[slot].get()).assume_init_read() };

        // Mark the slot as empty.
        let old_status = channel
            .status
            .fetch_and(!mark_slot(slot, MARK_EMPTIED), Ordering::AcqRel);

        // Debug assertion to check the slot was in the READING or FILLED
        // status. The slot can be in the FILLED status if the sender tried
        // to mark this slot as TAKEN (01) after we marked it as READING
        // (10) (01 | 10 = 11 (FILLED)).
        debug_assert!(
            has_status(old_status, slot, READING) || has_status(old_status, slot, FILLED)
        );

        channel.wake_next_sender();

        return Ok(value);
    }

    if is_connected {
        Err(RecvError::Empty)
    } else {
        Err(RecvError::Disconnected)
    }
}

/// See [`Receiver::try_peek`].
fn try_peek<T>(channel: &Channel<T>) -> Result<&T, RecvError> {
    // See `try_recv` why we do this first.
    let is_connected = sender_count(channel.ref_count.load(Ordering::Relaxed)) > 0;

    let status = channel.status.load(Ordering::Acquire);
    let cap = channel.slots.len();
    let start = receiver_pos(status, cap);
    for slot in (0..cap).cycle().skip(start).take(cap) {
        if !is_filled(status, slot) {
            continue;
        }

        // SAFETY: we've acquired unique access to the slot above and we're
        // ensured the slot is filled.
        return Ok(unsafe { (*channel.slots[slot].get()).assume_init_ref() });
    }

    if is_connected {
        Err(RecvError::Empty)
    } else {
        Err(RecvError::Disconnected)
    }
}

impl<T: fmt::Debug> fmt::Debug for Receiver<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Receiver")
            .field("channel", &self.channel())
            .finish()
    }
}

// SAFETY: if the value can be send across thread than so can the channel.
unsafe impl<T: Send> Send for Receiver<T> {}
unsafe impl<T> Sync for Receiver<T> {}

impl<T: RefUnwindSafe> RefUnwindSafe for Receiver<T> {}
impl<T: RefUnwindSafe> UnwindSafe for Receiver<T> {}

impl<T> Unpin for Receiver<T> {}

impl<T> Drop for Receiver<T> {
    #[rustfmt::skip]
    fn drop(&mut self) {
        // First mark the receiver as dropped.
        // SAFETY: for the reasoning behind this ordering see `Arc::drop`.
        let old_ref_count = self.channel().ref_count.fetch_and(!RECEIVER_ALIVE, Ordering::Release);
        if has_manager(old_ref_count) {
            // If the channel has a manager we only mark the receiver as dropped
            // (above).
            return;
        }

        // If the channel doesn't have a manager we empty the channel. We do
        // this to support the use case were the channel holds a
        // `oneshot::Sender` and the receiver of the oneshot channel is holding
        // a `Sender` to this channel. Effectively this creates a cyclic drop
        // dependency: `Sender` -> `Channel` -> `oneshot::Sender` which blocks
        // `oneshot::Receiver::recv`. If the actor holding a `Sender` calls
        // `oneshot::Receiver::recv` it will wait for a response or until the
        // `oneshot::Sender` is dropped, while the actor is holding a `Sender`
        // to this channel. However if this `Receiver` is dropped it won't drop
        // the `oneshot::Sender` without the emptying below. This causes
        // `oneshot::Receiver::recv` to wait forever, while holding a `Sender`.
        while let Ok(msg) = self.try_recv() {
            drop(msg);
        }

        // Let all senders know the sender is disconnected.
        self.channel().wake_all_join();

        // If the previous value was `RECEIVER_ACCESS` it means that all senders
        // and the manager were all dropped, so we need to do the deallocating.
        let old_ref_count = self.channel().ref_count.fetch_and(!RECEIVER_ACCESS, Ordering::Release);
        if old_ref_count != RECEIVER_ACCESS {
            // Another sender is alive, can't deallocate yet.
            return;
        }

        // For the reasoning behind this ordering see `Arc::drop`.
        fence!(self.channel().ref_count, Ordering::Acquire);

        // Drop the memory.
        unsafe { drop(Box::from_raw(self.channel.as_ptr())) }
    }
}

/// [`Future`] implementation behind [`Receiver::recv`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct RecvValue<'r, T> {
    channel: &'r Channel<T>,
}

impl<'r, T> Future for RecvValue<'r, T> {
    type Output = Option<T>;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context) -> Poll<Self::Output> {
        match try_recv(self.channel) {
            Ok(value) => Poll::Ready(Some(value)),
            Err(RecvError::Empty) => {
                // The channel is empty, we'll set the waker.
                if !self.channel.receiver_waker.register(ctx.waker()) {
                    // Waker already set.
                    return Poll::Pending;
                }

                // But it could be the case that a sender send a value in the
                // time between we last checked and we actually marked ourselves
                // as needing a wake up, so we need to check again.
                match try_recv(self.channel) {
                    Ok(value) => Poll::Ready(Some(value)),
                    // The `Sender` will wake us when a new message is send.
                    Err(RecvError::Empty) => Poll::Pending,
                    Err(RecvError::Disconnected) => Poll::Ready(None),
                }
            }
            Err(RecvError::Disconnected) => Poll::Ready(None),
        }
    }
}

impl<'r, T> Unpin for RecvValue<'r, T> {}

/// [`Future`] implementation behind [`Receiver::peek`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct PeekValue<'r, T> {
    channel: &'r Channel<T>,
}

impl<'r, T> Future for PeekValue<'r, T> {
    type Output = Option<&'r T>;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context) -> Poll<Self::Output> {
        match try_peek(self.channel) {
            Ok(value) => Poll::Ready(Some(value)),
            Err(RecvError::Empty) => {
                // The channel is empty, we'll set the waker.
                if !self.channel.receiver_waker.register(ctx.waker()) {
                    // Waker already set.
                    return Poll::Pending;
                }

                // But it could be the case that a sender send a value in the
                // time between we last checked and we actually marked ourselves
                // as needing a wake up, so we need to check again.
                match try_peek(self.channel) {
                    Ok(value) => Poll::Ready(Some(value)),
                    // The `Sender` will wake us when a new message is send.
                    Err(RecvError::Empty) => Poll::Pending,
                    Err(RecvError::Disconnected) => Poll::Ready(None),
                }
            }
            Err(RecvError::Disconnected) => Poll::Ready(None),
        }
    }
}

impl<'r, T> Unpin for PeekValue<'r, T> {}

/// Channel internals shared between zero or more [`Sender`]s, zero or one
/// [`Receiver`] and zero or one [`Manager`].
struct Channel<T> {
    inner: Inner,
    /// The slots in the channel, see `status` for what slots are used/unused.
    slots: [UnsafeCell<MaybeUninit<T>>],
}

/// Inner data of [`Channel`].
///
/// This is only in a different struct to calculate the `Layout` of `Channel`,
/// see [`Channel::new`].
struct Inner {
    /// Status of the slots.
    ///
    /// This contains the status of the slots. Each status consists of
    /// [`STATUS_BITS`] bits to describe if the slot is taken or not.
    ///
    /// The first `STATUS_BITS * MAX_CAP` bits are the statuses for the `slots`
    /// field. The remaining bits are used by the `Sender` to indicate its
    /// current reading position (modulo [`MAX_CAP`]).
    status: AtomicU64,
    /// The number of senders alive. If the [`RECEIVER_ALIVE`] bit is set the
    /// [`Receiver`] is alive. If the [`MANAGER_ALIVE`] bit is the [`Manager`]
    /// is alive.
    ref_count: AtomicUsize,
    sender_wakers: Mutex<Vec<task::Waker>>,
    join_wakers: Mutex<Vec<task::Waker>>,
    receiver_waker: WakerRegistration,
}

// SAFETY: if the value can be send across thread than so can the channel.
unsafe impl<T: Send> Send for Channel<T> {}

unsafe impl<T> Sync for Channel<T> {}

impl<T> Channel<T> {
    /// Allocates a new `Channel` on the heap.
    ///
    /// `capacity` must small enough to ensure each slot has 2 bits for the
    /// status, while ensuring that the remaining bits can store `capacity` (in
    /// binary) to keep track of the reading position. This means following must
    /// hold true where $N is capacity: `2 ^ (64 - ($N * 2)) >= $N`. The maximum
    /// is 29.
    ///
    /// Marks a single [`Receiver`] and [`Sender`] as alive.
    fn new(capacity: usize) -> NonNull<Channel<T>> {
        assert!(capacity >= MIN_CAP, "capacity can't be zero");
        assert!(capacity <= MAX_CAP, "capacity too large");

        // Allocate some raw bytes.
        // SAFETY: returns an error on arithmetic overflow, but it should be OK
        // with a capacity <= MAX_CAP.
        let (layout, _) = Layout::array::<UnsafeCell<MaybeUninit<T>>>(capacity)
            .and_then(|slots_layout| Layout::new::<Inner>().extend(slots_layout))
            .unwrap();
        // SAFETY: we check if the allocation is successful.
        let ptr = unsafe { alloc(layout) };
        if ptr.is_null() {
            handle_alloc_error(layout);
        }
        let ptr = ptr::slice_from_raw_parts_mut(ptr.cast::<T>(), capacity) as *mut Channel<T>;

        // Initialise all fields (that need it).
        unsafe {
            ptr::addr_of_mut!((*ptr).inner.status).write(AtomicU64::new(0));
            ptr::addr_of_mut!((*ptr).inner.ref_count).write(AtomicUsize::new(
                RECEIVER_ALIVE | RECEIVER_ACCESS | SENDER_ACCESS | 1,
            ));
            ptr::addr_of_mut!((*ptr).inner.sender_wakers).write(Mutex::new(Vec::new()));
            ptr::addr_of_mut!((*ptr).inner.join_wakers).write(Mutex::new(Vec::new()));
            ptr::addr_of_mut!((*ptr).inner.receiver_waker).write(WakerRegistration::new());
        }

        // SAFETY: checked if the pointer is null above.
        unsafe { NonNull::new_unchecked(ptr) }
    }

    /// Returns the next `task::Waker` to wake, if any.
    fn wake_next_sender(&self) {
        let waker = {
            let mut sender_wakers = self.sender_wakers.lock().unwrap();
            (!sender_wakers.is_empty()).then(|| sender_wakers.swap_remove(0))
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    /// Wakes all wakers waiting on the sender to disconnect.
    fn wake_all_join(&self) {
        let wakers = take(&mut *self.join_wakers.lock().unwrap());
        for waker in wakers {
            waker.wake();
        }
    }

    /// Wake the `Receiver`.
    fn wake_receiver(&self) {
        self.receiver_waker.wake();
    }
}

// NOTE: this is here so we don't have to type `self.channel().inner`
// everywhere.
impl<T> Deref for Channel<T> {
    type Target = Inner;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

#[allow(clippy::missing_fields_in_debug)]
impl<T> fmt::Debug for Channel<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let status = self.status.load(Ordering::Relaxed);
        let ref_count = self.ref_count.load(Ordering::Relaxed);
        let sender_count = sender_count(ref_count);
        let recv_pos = receiver_pos(status, self.slots.len());
        let mut slots = [""; MAX_CAP];
        for n in 0..self.slots.len() {
            slots[n] = dbg_status(slot_status(status, n));
        }
        let slots = &slots[..self.slots.len()];
        f.debug_struct("Channel")
            .field("senders_alive", &sender_count)
            .field("receiver_alive", &has_receiver(ref_count))
            .field("manager_alive", &has_manager(ref_count))
            .field("receiver_position", &recv_pos)
            .field("slots", &slots)
            .finish()
    }
}

impl<T> Drop for Channel<T> {
    fn drop(&mut self) {
        // SAFETY: we have unique access, per the mutable reference, so relaxed
        // is fine.
        let status: u64 = self.status.load(Ordering::Relaxed);
        for slot in 0..self.slots.len() {
            if is_filled(status, slot) {
                // SAFETY: we have unique access to the slot and we've checked
                // above whether or not the slot is filled.
                unsafe { self.slots[slot].get_mut().assume_init_drop() };
            }
        }
    }
}

/// Manager of a channel.
///
/// A channel manager can be used to create [`Sender`]s and [`Receiver`]s for a
/// channel, without having access to either. Its made for the following use
/// case: restarting an actor which takes ownership of the `Receiver` and
/// crashes, and to restart the actor we need another `Receiver`. Using the
/// manager a new `Receiver` can be created, ensuring only a single `Receiver`
/// is alive at any given time.
pub struct Manager<T> {
    channel: NonNull<Channel<T>>,
}

/// Error returned by [`Manager::new_receiver`] if a receiver is already
/// connected.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct ReceiverConnected;

impl fmt::Display for ReceiverConnected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad("receiver already connected")
    }
}

impl Error for ReceiverConnected {}

impl<T> Manager<T> {
    /// Create a small bounded channel with a `Manager`.
    ///
    /// Same as [`new_small`] but with a `Manager`.
    pub fn new_small_channel() -> (Manager<T>, Sender<T>, Receiver<T>) {
        Manager::new_channel(SMALL_CAP)
    }

    /// Create a bounded channel with a `Manager`.
    ///
    /// Same as [`new`] but with a `Manager`.
    pub fn new_channel(capacity: usize) -> (Manager<T>, Sender<T>, Receiver<T>) {
        let (sender, receiver) = new(capacity);
        let old_count = sender
            .channel()
            .ref_count
            .fetch_or(MANAGER_ALIVE | MANAGER_ACCESS, Ordering::Relaxed);
        debug_assert!(!has_manager(old_count));
        let manager = Manager {
            channel: sender.channel,
        };
        (manager, sender, receiver)
    }

    /// Create a new [`Sender`].
    ///
    /// # Safety
    ///
    /// See the [safety nodes] on `Sender`'s [`Clone`] implemenation, the same
    /// conditions apply here.
    ///
    /// [safety nodes]: struct.Sender.html#impl-Clone
    pub fn new_sender(&self) -> Sender<T> {
        // For the reasoning behind this relaxed ordering see `Arc::clone`.
        let old_ref_count = self.channel().ref_count.fetch_add(1, Ordering::Relaxed);
        if old_ref_count & SENDER_ACCESS != 0 {
            _ = self
                .channel()
                .ref_count
                .fetch_or(SENDER_ACCESS, Ordering::Relaxed);
        }
        Sender {
            channel: self.channel,
        }
    }

    /// Attempt to create a new [`Receiver`].
    ///
    /// This will fail if there already is a receiver.
    pub fn new_receiver(&self) -> Result<Receiver<T>, ReceiverConnected> {
        let old_count = self
            .channel()
            .ref_count
            .fetch_or(RECEIVER_ALIVE, Ordering::AcqRel);
        if has_receiver(old_count) {
            Err(ReceiverConnected)
        } else {
            // No receiver was connected so its safe to create one.
            debug_assert!(old_count & RECEIVER_ACCESS != 0);
            Ok(Receiver {
                channel: self.channel,
            })
        }
    }

    /// Returns the id of the channel.
    pub fn id(&self) -> Id {
        Id(self.channel.as_ptr().cast_const().cast::<()>() as usize)
    }

    fn channel(&self) -> &Channel<T> {
        unsafe { self.channel.as_ref() }
    }
}

impl<T> fmt::Debug for Manager<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Manager")
            .field("channel", &self.channel())
            .finish()
    }
}

// SAFETY: if the value can be send across thread than so can the channel.
unsafe impl<T: Send> Send for Manager<T> {}

unsafe impl<T> Sync for Manager<T> {}

impl<T> Unpin for Manager<T> {}

impl<T> Drop for Manager<T> {
    #[rustfmt::skip]
    fn drop(&mut self) {
        // First mark the manager as dropped.
        // SAFETY: for the reasoning behind this ordering see `Arc::drop`.
        let old_ref_count = self.channel().ref_count.fetch_and(!MANAGER_ALIVE, Ordering::Release);
        if has_receiver(old_ref_count) {
            // If the channel has a receiver we only mark the manager as dropped
            // (above).
            _ = self.channel().ref_count.fetch_and(!MANAGER_ACCESS, Ordering::Release);
            return;
        }

        debug_assert!(!has_receiver(old_ref_count));
        debug_assert!(old_ref_count & RECEIVER_ACCESS != 0);
        // NOTE: because `RECEIVER_ACCESS` bit is still set we don't have to set
        // the `RECEIVER_ALIVE` bit (as the receiver will dropped at the end of
        // the function).
        let receiver = Receiver { channel: self.channel };

        _ = self.channel().ref_count.fetch_and(!MANAGER_ACCESS, Ordering::Release);
        // Let the receiver do the cleanup.
        drop(receiver);
    }
}

/// Identifier of a channel.
///
/// This type can be created by calling [`Sender::id`] or [`Receiver::id`] and
/// be used to identify channels. Its only use case is to compare two ids with
/// one another, if two id are the same the sender(s) and receiver(s) point to
/// the same channel.
///
/// # Notes
///
/// The id is only valid for the lifetime of the channel. Once the channel is
/// dropped all ids of the channel are invalidated and might return incorrect
/// results after.
///
/// The methods [`Sender::same_channel`] and [`Sender::sends_to`] should be
/// preferred over using this type as they are less error-prone.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Id(usize);

impl Id {
    #[doc(hidden)] // Not part of the stable API.
    pub const fn as_usize(self) -> usize {
        self.0
    }
}
