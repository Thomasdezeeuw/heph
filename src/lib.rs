//! Bounded capacity channel.
//!
//! The channel is a multi-producer, single-consumer (MPSC) first in, first
//! out (FIFO) bounded queue. It is designed to be used as inbox for actors,
//! following the [actor model].
//!
//! [actor model]: https://en.wikipedia.org/wiki/Actor_model
//!
//! # Examples
//!
//! Simple creation of a channel and sending a message over it.
//!
//! ```
//! use std::thread;
//!
//! use inbox::RecvError;
//!
//! // Create a new small channel.
//! let (mut sender, mut receiver) = inbox::new_small();
//!
//! let sender_handle = thread::spawn(move || {
//!     if let Err(err) = sender.try_send("Hello world!".to_owned()) {
//!         panic!("Failed to send value: {}", err);
//!     }
//! });
//!
//! let receiver_handle = thread::spawn(move || {
//!     # thread::sleep(std::time::Duration::from_millis(1)); // Don't waste cycles.
//!     // NOTE: this is just an example don't actually use a loop like this, it
//!     // will waste CPU cycles when the channel is empty!
//!     loop {
//!         match receiver.try_recv() {
//!             Ok(value) => println!("Got a value: {}", value),
//!             Err(RecvError::Empty) => continue,
//!             Err(RecvError::Disconnected) => break,
//!         }
//!     }
//! });
//!
//! sender_handle.join().unwrap();
//! receiver_handle.join().unwrap();
//! ```

// FIXME: what do we do with SendValue Futures being pulled multiple times;
// creating multiple waker in the list, or when it can send a value after adding
// the waker. The Sender will wake the receiver, but the Future has already
// complete, so any other SendValue Future will not be awoken even though there
// is a slot available.

// TODO: support larger channel, with more slots.

#![feature(maybe_uninit_extra, maybe_uninit_ref, min_const_generics)]
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

use std::cell::UnsafeCell;
use std::error::Error;
use std::future::Future;
use std::mem::{size_of, MaybeUninit};
use std::pin::Pin;
use std::ptr::NonNull;
use std::sync::atomic::{fence, AtomicBool, AtomicPtr, AtomicUsize, Ordering};
use std::task::{self, Poll};
use std::{fmt, ptr};

use parking_lot::{const_rwlock, RwLock, RwLockUpgradableReadGuard};

#[cfg(test)]
mod tests;

/// Create a small bounded channel.
pub fn new_small<T>() -> (Sender<T>, Receiver<T>) {
    let channel = NonNull::from(Box::leak(Box::new(Channel::new())));
    let send = Sender {
        channel: channel.clone(),
    };
    let recv = Receiver { channel };
    (send, recv)
}

/// Bit mask to mark the receiver as alive.
const RECEIVER_ALIVE: usize = 1 << (size_of::<usize>() * 8 - 1);
/// Bit mask to mark the manager as alive.
const MANAGER_ALIVE: usize = 1 << (size_of::<usize>() * 8 - 2);

const LEN: usize = 8;

// Bits to mark the status of a slot.
const STATUS_BITS: usize = 2; // Number of bits used per slot.
const STATUS_MASK: usize = (1 << STATUS_BITS) - 1;
#[cfg(test)]
const ALL_STATUSES_MASK: usize = (1 << (LEN * STATUS_BITS)) - 1;
// The possible statuses of a slot.
const EMPTY: usize = 0b00; // Slot is empty (initial state).
const TAKEN: usize = 0b01; // `Sender` acquired write access, currently writing.
const FILLED: usize = 0b11; // `Sender` wrote a value into the slot.
const READING: usize = 0b10; // A `Receiver` is reading from the slot.

// Status transitions.
const MARK_TAKEN: usize = 0b01; // OR to go from EMPTY -> TAKEN.
const MARK_FILLED: usize = 0b11; // OR to go from TAKEN -> FILLED.
const MARK_READING: usize = 0b01; // XOR to go from FILLED -> READING.
const MARK_EMPTIED: usize = 0b11; // ! AND to go from FILLED or READING -> EMPTY.

/// Returns `true` if `slot` in `status` is empty.
#[inline(always)]
fn is_available(status: usize, slot: usize) -> bool {
    has_status(status, slot, EMPTY)
}

/// Returns `true` if `slot` in `status` is filled.
#[inline(always)]
fn is_filled(status: usize, slot: usize) -> bool {
    has_status(status, slot, FILLED)
}

/// Returns `true` if `slot` (in `status`) equals the `expected` status.
#[inline(always)]
fn has_status(status: usize, slot: usize, expected: usize) -> bool {
    slot_status(status, slot) == expected
}

/// Returns the `STATUS_BITS` for `slot` in `status`.
#[inline(always)]
fn slot_status(status: usize, slot: usize) -> usize {
    debug_assert!(slot <= LEN);
    (status >> (STATUS_BITS * slot)) & STATUS_MASK
}

/// Creates a mask to transition `slot` using `transition`. `transition` must be
/// one of the `MARK_*` constants.
#[inline(always)]
fn mark_slot(slot: usize, transition: usize) -> usize {
    debug_assert!(slot <= LEN);
    transition << (STATUS_BITS * slot)
}

/// Returns a string name for the `slot_status`.
fn dbg_status(slot_status: usize) -> &'static str {
    match slot_status {
        EMPTY => "EMPTY",
        TAKEN => "TAKEN",
        FILLED => "FILLED",
        READING => "READING",
        _ => "INVALID",
    }
}

// Bits to mark the position of the receiver.
const POS_BITS: usize = 3; // Must be `2 ^ POS_BITS == LEN`.
const POS_MASK: usize = (1 << POS_BITS) - 1;
const MARK_NEXT_POS: usize = 1 << (STATUS_BITS * LEN); // Add to increase position by 1.

/// Returns the position of the receiver. Will be in 0..LEN range.
#[inline(always)]
fn receiver_pos(status: usize) -> usize {
    status >> (STATUS_BITS * LEN) & POS_MASK
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
    pub fn try_send(&mut self, value: T) -> Result<(), SendError<T>> {
        if !self.is_connected() {
            return Err(SendError::Disconnected(value));
        }

        let channel = self.channel();
        // NOTE: relaxed ordering here is ok because we acquire unique
        // permission to write to the slot later before writing to it. Something
        // we have to do no matter the ordering.
        let mut status: usize = channel.status.load(Ordering::Relaxed);
        let start = receiver_pos(status);
        for slot in (0..LEN).cycle().skip(start).take(LEN) {
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

            // Safety: we've acquired the slot above so we're ensured unique
            // access to the slot.
            unsafe {
                let _ = (&mut *channel.slots[slot].get()).write(value);
            }

            // Now we've writing to the slot we can mark it slot as filled.
            let old_status = channel
                .status
                .fetch_or(mark_slot(slot, MARK_FILLED), Ordering::AcqRel);
            // Debug assertion to check the slot was in the TAKEN status.
            debug_assert!(has_status(old_status, slot, TAKEN));

            channel.wake_receiver();

            return Ok(());
        }

        Err(SendError::Full(value))
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
    pub fn send<'s>(&'s mut self, value: T) -> SendValue<'s, T> {
        SendValue {
            sender: self,
            value: Some(value),
        }
    }

    /// Returns `true` if the [`Receiver`] and the [`Manager`] are disconnected.
    ///
    /// # Notes
    ///
    /// Unlike [`Receiver::is_connected`] this method takes the [`Manager`] into
    /// account. This is done to support the use case in which an actor is
    /// restarted and a new receiver is created for it.
    pub fn is_connected(&self) -> bool {
        // Relaxed is fine here since there is always a bit of a race condition
        // when using this method (and then doing something based on it).
        self.channel().ref_count.load(Ordering::Relaxed) & (RECEIVER_ALIVE | MANAGER_ALIVE) != 0
    }

    /// Returns `true` if the [`Manager`] is connected.
    pub fn has_manager(&self) -> bool {
        // Relaxed is fine here since there is always a bit of a race condition
        // when using this method (and then doing something based on it).
        self.channel().ref_count.load(Ordering::Relaxed) & MANAGER_ALIVE != 0
    }

    /// Returns `true` if senders send into the same channel.
    pub fn same_channel(&self, other: &Sender<T>) -> bool {
        self.channel == other.channel
    }

    /// Returns `true` if this sender send to the `receiver`.
    pub fn sends_to(&self, receiver: &Receiver<T>) -> bool {
        self.channel == receiver.channel
    }

    /// Returns `true` if this is the only sender alive.
    fn only_sender(&self) -> bool {
        // Relaxed is fine here since there is always a bit of a race condition
        // when using this method (and then doing something based on it).
        self.channel().ref_count.load(Ordering::Relaxed) & !(RECEIVER_ALIVE | MANAGER_ALIVE) == 1
    }

    fn channel(&self) -> &Channel<T> {
        unsafe { self.channel.as_ref() }
    }
}

/// # Safety
///
/// Only `2 ^ 30` (a billion) `Sender`s may be alive concurrently, more then
/// enough for all practical use cases.
impl<T> Clone for Sender<T> {
    fn clone(&self) -> Sender<T> {
        // For the reasoning behind this relaxed ordering see `Arc::clone`.
        let _ = self.channel().ref_count.fetch_add(1, Ordering::Relaxed);
        Sender {
            channel: self.channel,
        }
    }
}

impl<T: fmt::Debug> fmt::Debug for Sender<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Sender")
            .field("channel", self.channel())
            .finish()
    }
}

// Safety: if the value can be send across thread than so can the channel.
unsafe impl<T: Send> Send for Sender<T> {}

unsafe impl<T> Sync for Sender<T> {}

impl<T> Unpin for Sender<T> {}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        // If we're the last sender being dropped wake the receiver.
        //
        // NOTE: there is a race condition between the `wake` and `fetch_sub`
        // below: in between those calls the receiver could run (after we
        // woke it) and see we're still connected and sleep (return
        // `Poll::Pending`) again. This can't be fixed.
        // The alternative would be to call `fetch_sub` on the `ref_count`
        // before waking, ensuring the count is valid once the `Sender` runs,
        // however that opens another race condition in which the `Sender` can
        // be dropped and deallocate the `Channel` memory, after which we'll
        // access it to wake the `Sender`. Basically we're choosing the least
        // worse of the two race conditions in which in the worst case scenario
        // is that the `Sender` loses a wake-up notification, but it doesn't
        // have any memory unsafety.
        if self.only_sender() {
            self.channel().wake_receiver();
        }

        // If the previous value was `1` it means that the receiver was dropped
        // as well as all other senders, the receiver and the manager, so we
        // need to do the deallocating.
        //
        // Safety: for the reasoning behind this ordering see `Arc::drop`.
        if self.channel().ref_count.fetch_sub(1, Ordering::Release) != 1 {
            return;
        }

        // For the reasoning behind this ordering see `Arc::drop`.
        fence(Ordering::Acquire);

        // Drop the memory.
        unsafe { drop(Box::from_raw(self.channel.as_ptr())) }
    }
}

/// [`Future`] implementation behind [`Sender::send`].
#[derive(Debug)]
pub struct SendValue<'s, T> {
    sender: &'s mut Sender<T>,
    value: Option<T>,
}

impl<'s, T> Future for SendValue<'s, T> {
    type Output = Result<(), T>;

    fn poll(mut self: Pin<&mut Self>, ctx: &mut task::Context) -> Poll<Self::Output> {
        let value = self
            .value
            .take()
            .expect("SendValue polled after completion");

        // First we try to send the value, if this succeeds we don't have to
        // allocate in the waker list.
        match self.sender.try_send(value) {
            Ok(()) => Poll::Ready(Ok(())),
            Err(SendError::Full(value)) => {
                // The channel is full, we'll register ourselves as wanting to
                // be woken once a slot opens up.
                self.sender.channel().add_waker(ctx.waker().clone());

                // But it could be the case that the received received a value
                // in the time after we tried to send the value and before we
                // added the our waker to list. So we try to send a value again
                // to ensure we don't awoken and the channel has a slot
                // available.
                match self.sender.try_send(value) {
                    Ok(()) => Poll::Ready(Ok(())),
                    Err(SendError::Full(value)) => {
                        // Channel is still full, we'll have to wait.
                        self.value = Some(value);
                        Poll::Pending
                    }
                    Err(SendError::Disconnected(value)) => Poll::Ready(Err(value)),
                }
            }
            Err(SendError::Disconnected(value)) => Poll::Ready(Err(value)),
        }
    }
}

impl<T> Unpin for SendValue<'_, T> {}

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
    pub fn try_recv(&mut self) -> Result<T, RecvError> {
        let channel = self.channel();
        // Since we substract from the `status` this will overflow at some
        // point. But `fetch_add` wraps-around on overflow, so the position will
        // "reset" itself to 0. The status bits will not be touched (even on
        // wrap-around).
        let mut status = channel.status.fetch_add(MARK_NEXT_POS, Ordering::AcqRel);
        let start = receiver_pos(status);
        for slot in (0..LEN).cycle().skip(start).take(LEN) {
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

            // Safety: we've acquired unique access the slot above and we're
            // ensured the slot is filled.
            let value = unsafe { (&mut *channel.slots[slot].get()).read() };

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

            if let Some(waker) = self.channel().next_waker() {
                waker.wake()
            }

            return Ok(value);
        }

        if !self.is_connected() {
            Err(RecvError::Disconnected)
        } else {
            Err(RecvError::Empty)
        }
    }

    /// Returns a future that receives a value from the channel, waiting if the
    /// channel is empty.
    ///
    /// If the returned [`Future`] returns `None` it means all [`Receiver`]s are
    /// [disconnected]. This is the same error as [`RecvError::Disconnected`].
    /// [`RecvError::Empty`] will never be returned, the `Future` will return
    /// [`Poll::Pending`] instead.
    ///
    /// [disconnected]: Receiver::is_connected
    pub fn recv<'s>(&'s mut self) -> RecvValue<'s, T> {
        RecvValue { receiver: self }
    }

    /// Create a new [`Sender`] that sends to this channel.
    ///
    /// # Safety
    ///
    /// The same restrictions apply to this function as they do to
    /// [`Sender::clone`].
    ///
    /// [`Sender::clone`]: struct.Sender.html#impl-Clone
    pub fn new_sender(&self) -> Sender<T> {
        // For the reasoning behind this relaxed ordering see `Arc::clone`.
        let _ = self.channel().ref_count.fetch_add(1, Ordering::Relaxed);
        Sender {
            channel: self.channel,
        }
    }

    /// Returns `true` if all [`Sender`]s are disconnected.
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
        self.channel().ref_count.load(Ordering::Relaxed) & !(RECEIVER_ALIVE | MANAGER_ALIVE) > 0
    }

    /// Returns `true` if the [`Manager`] is connected.
    pub fn has_manager(&self) -> bool {
        // Relaxed is fine here since there is always a bit of a race condition
        // when using this method (and then doing something based on it).
        self.channel().ref_count.load(Ordering::Relaxed) & MANAGER_ALIVE != 0
    }

    /// Set the receiver waker to `waker`, if they are different.
    fn set_waker(&mut self, waker: &task::Waker) {
        let receiver_waker = self.channel().receiver_waker.upgradable_read();

        if let Some(receiver_waker) = &*receiver_waker {
            if receiver_waker.will_wake(waker) {
                return;
            }
        }

        let waker = Some(waker.clone());
        let mut receiver_waker = RwLockUpgradableReadGuard::upgrade(receiver_waker);
        *receiver_waker = waker;
    }

    /// Mark that the `Receiver` needs to be woken up.
    fn need_receiver_wakeup(&self) {
        self.channel()
            .receiver_needs_wakeup
            .store(true, Ordering::Relaxed);
    }

    fn channel(&self) -> &Channel<T> {
        unsafe { self.channel.as_ref() }
    }
}

impl<T: fmt::Debug> fmt::Debug for Receiver<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Receiver")
            .field("channel", self.channel())
            .finish()
    }
}

// Safety: if the value can be send across thread than so can the channel.
unsafe impl<T: Send> Send for Receiver<T> {}

unsafe impl<T> Sync for Receiver<T> {}

impl<T> Unpin for Receiver<T> {}

impl<T> Drop for Receiver<T> {
    #[rustfmt::skip] // For the if statement, its complicated enough.
    fn drop(&mut self) {
        // If the previous value was `RECEIVER_ALIVE` it means that all senders
        // and the manager were dropped, so we need to do the deallocating.
        //
        // Safety: for the reasoning behind this ordering see `Arc::drop`.
        if self.channel().ref_count.fetch_and(!RECEIVER_ALIVE, Ordering::Release) != RECEIVER_ALIVE {
            return;
        }

        // For the reasoning behind this ordering see `Arc::drop`.
        fence(Ordering::Acquire);

        // Drop the memory.
        unsafe { drop(Box::from_raw(self.channel.as_ptr())) }
    }
}

/// [`Future`] implementation behind [`Receiver::recv`].
#[derive(Debug)]
pub struct RecvValue<'s, T> {
    receiver: &'s mut Receiver<T>,
}

impl<'s, T> Future for RecvValue<'s, T> {
    type Output = Option<T>;

    fn poll(mut self: Pin<&mut Self>, ctx: &mut task::Context) -> Poll<Self::Output> {
        match self.receiver.try_recv() {
            Ok(value) => Poll::Ready(Some(value)),
            Err(RecvError::Empty) => {
                // The channel is empty, we'll set the waker and register
                // ourselves as wanting to be woken once a value is added.
                self.receiver.set_waker(ctx.waker());
                self.receiver.need_receiver_wakeup();

                // But it could be the case that a sender send a value in the
                // time between we last checked and we actually marked ourselves
                // as needing a wake up, so we need to check again.
                match self.receiver.try_recv() {
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

impl<T> Unpin for RecvValue<'_, T> {}

/// Channel internals shared between zero or more [`Sender`]s, zero or one
/// [`Receiver`] and zero or one [`Manager`].
struct Channel<T> {
    /// Status of the slots.
    ///
    /// This contains the status of the slots. Each status consists of
    /// [`STATUS_BITS`] bits to describe if the slot is taken or not.
    ///
    /// The first `STATUS_BITS * LEN` bits are the statuses for the `slots`
    /// field. The remaining bits are used by the `Sender` to indicate its
    /// current reading position (modulo `LEN`).
    status: AtomicUsize,
    /// The slots in the channel, see `status` for what slots are used/unused.
    slots: [UnsafeCell<MaybeUninit<T>>; LEN],
    /// The number of senders alive. If the [`RECEIVER_ALIVE`] bit is set the
    /// [`Receiver`] is alive. If the [`MANAGER_ALIVE`] bit is the [`Manager`]
    /// is alive.
    ref_count: AtomicUsize,
    /// This is a linked list of `task::Waker`.
    ///
    /// If this is not null it must point to valid memory.
    sender_waker_tail: AtomicPtr<LinkedList>,
    /// `task::Waker` to wake the `Receiver`.
    receiver_waker: RwLock<Option<task::Waker>>,
    /// `true` if `receiver_waker` needs to be awoken after sending a message,
    /// see the `wake_receiver` method.
    receiver_needs_wakeup: AtomicBool,
}

/// Atomic linked list.
///
/// This list can only add items to the tail and remove them from the head. Only
/// a single remover (the `Receiver`) is allowed.
// TODO: reuse `LinkedList` allocations.
#[derive(Debug)]
struct LinkedList {
    waker: task::Waker,
    prev: AtomicPtr<Self>,
}

impl<T> Channel<T> {
    /// Marks a single [`Receiver`] and [`Sender`] as alive.
    const fn new() -> Channel<T> {
        Channel {
            slots: [
                UnsafeCell::new(MaybeUninit::uninit()),
                UnsafeCell::new(MaybeUninit::uninit()),
                UnsafeCell::new(MaybeUninit::uninit()),
                UnsafeCell::new(MaybeUninit::uninit()),
                UnsafeCell::new(MaybeUninit::uninit()),
                UnsafeCell::new(MaybeUninit::uninit()),
                UnsafeCell::new(MaybeUninit::uninit()),
                UnsafeCell::new(MaybeUninit::uninit()),
            ],
            status: AtomicUsize::new(0),
            ref_count: AtomicUsize::new(RECEIVER_ALIVE | 1),
            sender_waker_tail: AtomicPtr::new(ptr::null_mut()),
            receiver_waker: const_rwlock(None),
            receiver_needs_wakeup: AtomicBool::new(false),
        }
    }

    /// Returns the next `task::Waker` to wake, if any.
    fn next_waker(&self) -> Option<task::Waker> {
        // Safety: relaxed is ok because if `SendValue` will try sending the
        // value again after it adds its waker to the list.
        let tail_ptr = self.sender_waker_tail.load(Ordering::Relaxed);
        if tail_ptr.is_null() {
            return None;
        }

        // The links may only be accessed by the `Receiver` after they're added
        // to the list by the `Sender`. Because of this we can safely use atomic
        // `store` operations with `Relaxed` ordering. However this is not true
        // for the `Channel.sender_waker_tail` field, as the `Sender`s may write
        // to this as well. We handle this case here using atomic
        // `compare_exchange` and use the cheaper `store` operations in the loop
        // below once we have unique access.
        let tail_prev_ptr: *mut LinkedList = unsafe { (*tail_ptr).prev.load(Ordering::Relaxed) };
        if tail_prev_ptr.is_null() {
            // Only a single item in the list, overwrite
            // `Channel.sender_waker_tail`.
            let res = self.sender_waker_tail.compare_exchange(
                tail_ptr,
                ptr::null_mut(),
                Ordering::AcqRel,
                Ordering::Relaxed,
            );
            match res {
                // Successfully removed the link from the list.
                Ok(ptr) => {
                    let link = unsafe { Box::from_raw(ptr) };
                    return Some(link.waker);
                }
                // Failed update the pointer, this means there are now at least
                // two links in the list, so we can continue below.
                Err(..) => {}
            }
        }

        // Safety: For pointers we checked above if they are null.
        let mut parent: &LinkedList = unsafe { &*tail_ptr };
        let mut link: &LinkedList = unsafe { &*tail_prev_ptr };
        loop {
            // `Relaxed` ordering is ok here as we have unique access.
            let prev_ptr = link.prev.load(Ordering::Relaxed);
            if prev_ptr.is_null() {
                // Last link, so remove itself from the parent.
                // A `Relaxed` store is ok here since we're the only once who
                // have access to these links.
                parent.prev.store(ptr::null_mut(), Ordering::Relaxed);
                let link = unsafe { Box::from_raw(link as *const _ as *mut LinkedList) };
                return Some(link.waker);
            } else {
                // Not the end of the list yet.
                parent = link;
                // Safety: checked above if its null.
                link = unsafe { &*prev_ptr };
            }
        }
    }

    /// Adds `waker` to the list of wakers to wake.
    fn add_waker(&self, waker: task::Waker) {
        // Create a new link.
        let link = Box::into_raw(Box::new(LinkedList {
            waker,
            prev: AtomicPtr::new(ptr::null_mut()),
        }));

        // Relaxed is ok here because we use `AcqRel` in `compare_exchange`
        // below to ensure the tail didn't change.
        let mut prev_ptr = self.sender_waker_tail.load(Ordering::Relaxed);
        loop {
            // Update the `prev` link to the current tail.
            // Safety: we just create `link` above and so we're assured we have
            // unique access and the pointer is valid.
            unsafe {
                *(*link).prev.get_mut() = prev_ptr;
            }
            // Relaxed for failure is ok because on the next loop we use
            // `AcqRel` again.
            let res = self.sender_waker_tail.compare_exchange(
                prev_ptr,
                link,
                Ordering::AcqRel,
                Ordering::Relaxed,
            );
            match res {
                // Successfully stored our item in the list.
                Ok(..) => return,
                // Failed update the pointer and try again.
                Err(prev) => prev_ptr = prev,
            }
        }
    }

    /// Wake the `Receiver`.
    fn wake_receiver(&self) {
        if !self.receiver_needs_wakeup.load(Ordering::Relaxed) {
            // Receiver doesn't need a wake-up.
            return;
        }

        // Mark that we've woken the `Sender` and after actually wake the
        // `Sender`.
        if self.receiver_needs_wakeup.swap(false, Ordering::Relaxed) {
            if let Some(receiver_waker) = &*self.receiver_waker.read() {
                receiver_waker.wake_by_ref();
            }
        }
    }
}

impl<T> fmt::Debug for Channel<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let status = self.status.load(Ordering::Relaxed);
        let ref_count = self.ref_count.load(Ordering::Relaxed);
        let sender_count = ref_count & (!(RECEIVER_ALIVE | MANAGER_ALIVE));
        f.debug_struct("Channel")
            .field("senders_alive", &sender_count)
            .field("receiver_alive", &(ref_count & RECEIVER_ALIVE != 0))
            .field("manager_alive", &(ref_count & MANAGER_ALIVE != 0))
            .field(
                "slots",
                &[
                    dbg_status(slot_status(status, 0)),
                    dbg_status(slot_status(status, 1)),
                    dbg_status(slot_status(status, 2)),
                    dbg_status(slot_status(status, 3)),
                    dbg_status(slot_status(status, 4)),
                    dbg_status(slot_status(status, 5)),
                    dbg_status(slot_status(status, 6)),
                    dbg_status(slot_status(status, 7)),
                ],
            )
            .finish()
    }
}

impl<T> Drop for Channel<T> {
    fn drop(&mut self) {
        // Safety: we have unique access, per the mutable reference, so relaxed
        // is fine.
        let status: usize = self.status.load(Ordering::Relaxed);
        for slot in 0..LEN {
            if is_filled(status, slot) {
                // Safety: we have unique access to the slot and it's properly
                // aligned.
                unsafe { ptr::drop_in_place((&mut *self.slots[slot].get()).as_mut_ptr()) };
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
        let (sender, receiver) = new_small();
        let old_count = sender
            .channel()
            .ref_count
            .fetch_or(MANAGER_ALIVE, Ordering::Relaxed);
        assert!(old_count & MANAGER_ALIVE == 0);
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
        let _ = self.channel().ref_count.fetch_add(1, Ordering::Relaxed);
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
        if old_count & RECEIVER_ALIVE == 0 {
            // No receiver was connected so its safe to create one.
            Ok(Receiver {
                channel: self.channel,
            })
        } else {
            Err(ReceiverConnected)
        }
    }

    fn channel(&self) -> &Channel<T> {
        unsafe { self.channel.as_ref() }
    }
}

impl<T: fmt::Debug> fmt::Debug for Manager<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Manager")
            .field("channel", self.channel())
            .finish()
    }
}

// Safety: if the value can be send across thread than so can the channel.
unsafe impl<T: Send> Send for Manager<T> {}

unsafe impl<T> Sync for Manager<T> {}

impl<T> Unpin for Manager<T> {}

impl<T> Drop for Manager<T> {
    #[rustfmt::skip] // For the if statement, its complicated enough.
    fn drop(&mut self) {
        // If the previous value was `MANAGER_ALIVE` it means that all senders
        // and receivers were dropped, so we need to do the deallocating.
        //
        // Safety: for the reasoning behind this ordering see `Arc::drop`.
        if self.channel().ref_count.fetch_and(!MANAGER_ALIVE, Ordering::Release) != MANAGER_ALIVE {
            return;
        }

        // For the reasoning behind this ordering see `Arc::drop`.
        fence(Ordering::Acquire);

        // Drop the memory.
        unsafe { drop(Box::from_raw(self.channel.as_ptr())) }
    }
}
