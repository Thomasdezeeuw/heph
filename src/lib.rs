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

#![feature(box_into_raw_non_null, maybe_uninit_extra, maybe_uninit_ref)]
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
use std::sync::atomic::{fence, AtomicPtr, AtomicUsize, Ordering};
use std::task::{self, Poll};
use std::{fmt, ptr};

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

// TODO: support a version with more slots.
const LEN: usize = 8;

// Bits to mark the status of a slot.
const STATUS_BITS: usize = 2;
const STATUS_MASK: usize = (1 << STATUS_BITS) - 1;
#[cfg(test)]
const ALL_STATUSES_MASK: usize = (1 << (LEN * STATUS_BITS)) - 1;
const EMPTY: usize = 0b00; // Slot is empty (initial state).
const TAKEN: usize = 0b01; // A `Sender` acquired write access, currently writing.
const FILLED: usize = 0b11; // A `Sender` wrote a value into the slot.
const READING: usize = 0b10; // A `Receiver` is reading from the slot.
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
            // (00) we can reuse the slot.
            status = channel
                .status
                .fetch_or(TAKEN << (STATUS_BITS * slot), Ordering::AcqRel);
            if !is_available(status, slot) {
                // Another thread beat us to taking the slot.
                continue;
            }

            // Safety: we've acquired the slot above so we're ensured single
            // access to the slot.
            unsafe {
                let _ = (&mut *channel.slots[slot].get()).write(value);
            }

            // Mark the slot as filled.
            let old_status = channel
                .status
                .fetch_or(FILLED << (STATUS_BITS * slot), Ordering::AcqRel);
            // Debug assertion to check the slot was in the TAKEN status.
            debug_assert!(has_status(old_status, slot, TAKEN));
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
        let value = self.value.take().expect("Send polled after completion");
        match self.sender.try_send(value) {
            Ok(()) => Poll::Ready(Ok(())),
            Err(SendError::Full(value)) => {
                self.value = Some(value);
                LinkedList::add(
                    &self.sender.channel().sender_waker_tail,
                    ctx.waker().clone(),
                );
                Poll::Pending
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
            RecvError::Disconnected => f.pad("channel is empty and no senders are connected"),
        }
    }
}

impl Error for RecvError {}

impl<T> Receiver<T> {
    /// Attempts to receive a value from this channel.
    pub fn try_recv(&mut self) -> Result<T, RecvError> {
        let channel = self.channel();
        // Since we have substract from the `status` this will overflow at some
        // point. But `fetch_add` wraps-around on overflow, so the position will
        // "reset" itself to 0. The status bits will not be touched (even on
        // wrap-around).
        let mut status: usize = channel.status.fetch_add(MARK_NEXT_POS, Ordering::AcqRel);
        let start = receiver_pos(status);
        for slot in (0..LEN).cycle().skip(start).take(LEN) {
            if !is_filled(status, slot) {
                continue;
            }

            // Mark the slot as being read, see the `fetch_or` call in
            // `Sender::send` for
            status = channel
                .status
                .fetch_xor(MARK_READING << (STATUS_BITS * slot), Ordering::AcqRel);
            if !is_filled(status, slot) {
                // Slot isn't available after all.
                continue;
            }

            // Safety: we've acquired the slot above so we're ensured single
            // access to the slot.
            let value = unsafe { (&mut *channel.slots[slot].get()).read() };

            // Mark the slot as EMPTY.
            let old_status = channel
                .status
                .fetch_and(!(MARK_EMPTIED << (STATUS_BITS * slot)), Ordering::AcqRel);

            // Debug assertion to check the slot was in the READING or FILLED
            // status. The slot can be in the FILLED status if a sender tried to
            // mark this slot as TAKEN (01) after we marked it as READING (10)
            // (01 | 10 = 11 (FILLED)).
            debug_assert!(
                has_status(old_status, slot, READING) || has_status(old_status, slot, FILLED)
            );
            self.wake_next_sender();
            return Ok(value);
        }

        if !self.is_connected() {
            Err(RecvError::Disconnected)
        } else {
            Err(RecvError::Empty)
        }
    }

    fn wake_next_sender(&mut self) {
        if let Some(waker) = LinkedList::remove(&self.channel().sender_waker_tail) {
            waker.wake()
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

struct Channel<T> {
    /// Status of the slots.
    ///
    /// This contains the status of the slots. Each status consists of
    /// [`STATUS_BITS`] bits to describe if the slot is taken or not.
    ///
    /// TODO: expand doc.
    status: AtomicUsize,
    /// The slots in the channel, see `status` for what slots are used/unused.
    slots: [UnsafeCell<MaybeUninit<T>>; LEN],
    /// The number of senders alive. If the [`RECEIVER_ALIVE`] bit is set the
    /// [`Receiver`] is alive. If the [`MANAGER_ALIVE`] bit is the [`Manager`]
    /// is alive.
    ref_count: AtomicUsize,
    //receiver_waker: task::Waker,
    /// This is a linked list of `task::Waker`.
    ///
    /// If this is not null it must point to correct memory.
    sender_waker_tail: AtomicPtr<LinkedList<task::Waker>>,
}

impl<T> Channel<T> {
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
        }
    }
}

/// Atomic linked list.
///
/// This link can only add items to the tail and remove them from the head. Only
/// a single remover (receiver) is allowed.
struct LinkedList<T> {
    item: T,
    prev: AtomicPtr<LinkedList<T>>,
}

impl<T> LinkedList<T> {
    // TODO: reuse `LinkedList` allocation.
    #[allow(unused_mut, unused_variables)] // FIXME: remove.
    fn remove(tail: &AtomicPtr<LinkedList<T>>) -> Option<T> {
        // TODO: comment on ordering.

        let mut tail_ptr = tail.load(Ordering::Relaxed);
        if tail_ptr.is_null() {
            return None;
        }

        // As the `Sender`s may write to the `tail` of the list we treat it as a
        // special case so we can safely use `AtomicPtr::store` in the loop
        // below as the previous link are only allow to be modified by the
        // `Receiver` (of which there is only one).
        let mut prev_ptr = unsafe { &(*tail_ptr).prev.load(Ordering::Relaxed) };
        if prev_ptr.is_null() {
            // Only a single item in the list.
            let res = tail.compare_exchange(
                tail_ptr,
                ptr::null_mut(),
                Ordering::AcqRel,
                Ordering::Relaxed,
            );
            match res {
                // Successfully removed the link from the list.
                Ok(ptr) => {
                    let link = unsafe { Box::from_raw(ptr) };
                    return Some(link.item);
                }
                // Failed update the pointer, this means there are now at least
                // two links in the list, so we can continue below.
                Err(..) => {}
            }
        }

        let mut atomic_prev = unsafe { &(*tail_ptr).prev };
        loop {
            /*
            let prev_ptr = atomic_prev.load(Ordering::Relaxed);

            if !prev_ptr.is_null() {
                // List has another link.
                atomic_prev = unsafe { &(*prev_ptr).prev };
            }
            // Last link in the list.

            // Remove the link from the list, now we have unique access to
            // the link in `prev_ptr`.
            // FIXME: incorrect! Need its parent.
            atomic_prev.store(ptr::null_mut(), Ordering::Relaxed);
            */

            /*
            // prev_ptr is null -> atomic_ptr is last item.
            let res =
                tail.compare_exchange(ptr, ptr::null_mut(), Ordering::AcqRel, Ordering::Relaxed);
            match res {
                // Successfully stored our item in the list.
                Ok(..) => {
                    //
                    todo!()
                }
                // Failed update the pointer and try again.
                Err(prev) => prev_ptr = prev,
            }
            */

            // TODO: return tail_ptr.item.
            todo!()
        }
    }

    fn add(tail: &AtomicPtr<LinkedList<T>>, item: T) {
        // Create a new link.
        let link = Box::into_raw(Box::new(LinkedList {
            item,
            prev: AtomicPtr::new(ptr::null_mut()),
        }));

        // Relaxed is ok here because use `AcqRel` in `compare_exchange` to
        // ensure the tail didn't change.
        let mut prev_ptr = tail.load(Ordering::Relaxed);
        loop {
            // Update the `prev` link to the current tail.
            // Safety: we just create `link` above and so we're assured we have
            // unique access and the pointer is valid.
            unsafe {
                *(&mut *link).prev.get_mut() = prev_ptr;
            }
            // Relaxed for failure is ok because on the next loop we use
            // `AcqRel` again.
            let res = tail.compare_exchange(prev_ptr, link, Ordering::AcqRel, Ordering::Relaxed);
            match res {
                // Successfully stored our item in the list.
                Ok(..) => return,
                // Failed update the pointer and try again.
                Err(prev) => prev_ptr = prev,
            }
        }
    }
}

impl<T: fmt::Debug> fmt::Debug for Channel<T> {
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
        let status: usize = self.status.load(Ordering::Relaxed);
        for slot in 0..LEN {
            if is_filled(status, slot) {
                unsafe { drop((&mut *self.slots[slot].get()).read()) };
            }
        }
    }
}

/// Manager of a channel.
///
/// A channel manager can be used to create [`Sender`]s and [`Receiver`]s for a
/// channel, without have access to either. Its made for the following use case:
/// restarting an actor which takes ownership of the `Receiver` and crashes, and
/// to restart the actor we need another `Receiver`. Using the manager a new
/// `Receiver` can be created, ensuring only a single `Receiver` is alive at any
/// given time.
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
