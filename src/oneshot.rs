//! One-shot channel.
//!
//! The channel allows you to send a single value and that it. It does allow the
//! channel's allocation to be reused via [`Receiver::try_reset`]. It is
//! designed to be used for [Remote Procedure Calls (RPC)].
//!
//! [Remote Procedure Calls (RPC)]: https://en.wikipedia.org/wiki/Remote_procedure_call
//!
//!
//! # Examples
//!
//! Simple creation of a channel and sending a message over it.
//!
//! ```
//! use std::thread;
//!
//! use heph_inbox::oneshot::{RecvError, new_oneshot};
//!
//! // Create a new small channel.
//! let (sender, mut receiver) = new_oneshot();
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
//!             Ok((value, _)) => println!("Got a value: {}", value),
//!             Err(RecvError::NoValue) => continue,
//!             Err(RecvError::Disconnected) => break,
//!         }
//!     }
//! });
//!
//! sender_handle.join().unwrap();
//! receiver_handle.join().unwrap();
//! ```

use std::cell::UnsafeCell;
use std::fmt;
use std::future::Future;
use std::mem::{size_of, MaybeUninit};
use std::pin::Pin;
use std::ptr::{self, NonNull};
use std::sync::atomic::{AtomicU8, Ordering};
use std::task::{self, Poll};

use parking_lot::{const_mutex, Mutex};

/// Create a new one-shot channel.
pub fn new_oneshot<T>() -> (Sender<T>, Receiver<T>) {
    let shared = NonNull::from(Box::leak(Box::new(Shared::new())));
    (Sender { shared }, Receiver { shared })
}

/// Bits mask to mark the receiver as alive.
const RECEIVER_ALIVE: u8 = 1 << (size_of::<u8>() * 8 - 1);
/// Bit mask to mark the sender as alive.
const SENDER_ALIVE: u8 = 1 << (size_of::<u8>() * 8 - 2);
/// Bit mask to mark the sender still has access to the shared data.
const SENDER_ACCESS: u8 = 1 << (size_of::<u8>() * 8 - 3);

// Status of the message in `Shared`.
const EMPTY: u8 = 0;
const FILLED: u8 = 1;

// Status transitions.
const MARK_FILLED: u8 = 1; // ADD to go from EMPTY -> FILLED.

/// The sending half of the [one-shot channel].
///
/// This half can only be owned and used by one thread.
///
/// [one-shot channel]: crate::oneshot::new_oneshot
pub struct Sender<T> {
    // Safety: must always point to valid memory.
    shared: NonNull<Shared<T>>,
}

impl<T> Sender<T> {
    /// Attempts to send a `value` into the channel. If this returns an error it
    /// means the receiver has disconnected (has been dropped).
    pub fn try_send(self, value: T) -> Result<(), T> {
        if !self.is_connected() {
            return Err(value);
        }

        let shared = self.shared();

        // This is safe because we're the only sender.
        unsafe { ptr::write(shared.message.get(), MaybeUninit::new(value)) };

        // Mark the item as filled.
        let _ = shared.status.fetch_add(MARK_FILLED, Ordering::AcqRel);
        // TODO: check old status with `debug_assert`.

        // Note: we wake in the `Drop` impl.
        Ok(())
    }

    /// Returns `true` if the [`Receiver`] is connected.
    pub fn is_connected(&self) -> bool {
        // Relaxed is fine here since there is always a bit of a race condition
        // when using the method (and then doing something based on it).
        self.shared().status.load(Ordering::Relaxed) & RECEIVER_ALIVE != 0
    }

    /// Returns `true` if this sender sends to the `receiver`.
    pub fn sends_to(&self, receiver: &Receiver<T>) -> bool {
        self.shared == receiver.shared
    }

    /// Reference the shared data.
    fn shared(&self) -> &Shared<T> {
        // Safety: see `shared` field.
        unsafe { self.shared.as_ref() }
    }
}

impl<T> fmt::Debug for Sender<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Sender")
    }
}

// Safety: if the value can be send across thread than so can the channel.
unsafe impl<T: Send> Send for Sender<T> {}

unsafe impl<T> Sync for Sender<T> {}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        // Mark ourselves as dropped, but still holding access.
        let shared = self.shared();
        let old_status = shared.status.fetch_and(!SENDER_ALIVE, Ordering::AcqRel);

        if old_status & RECEIVER_ALIVE != 0 {
            // Receiver is still alive, so we need to wake it.
            if let Some(waker) = shared.receiver_waker.lock().take() {
                waker.wake();
            }
        }

        // Now mark that we don't have access anymore.
        let old_status = shared.status.fetch_and(!SENDER_ACCESS, Ordering::AcqRel);

        if old_status & RECEIVER_ALIVE != 0 {
            // Receiver is still alive, no need to drop.
            return;
        }

        // Drop the shared memory.
        unsafe { drop(Box::from_raw(self.shared.as_ptr())) }
    }
}

/// The receiving half of the [one-shot channel].
///
/// This half can only be owned and used by one thread.
///
/// [one-shot channel]: crate::oneshot::new_oneshot
pub struct Receiver<T> {
    // Safety: must always point to valid memory.
    shared: NonNull<Shared<T>>,
}

/// Error returned by [`Receiver::try_recv`].
#[derive(Debug, Eq, PartialEq)]
pub enum RecvError {
    /// No value is available, but the sender is still connected.
    NoValue,
    /// Sender is disconnected and no value is available.
    Disconnected,
}

impl fmt::Display for RecvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecvError::NoValue => f.write_str("no value available"),
            RecvError::Disconnected => f.write_str("sender disconnected"),
        }
    }
}

impl<T> Receiver<T> {
    /// Attempts to receive a value and reset the channel.
    ///
    /// If it succeeds it returns the value and resets the channel, returning a
    /// new [`Sender`] (which can send a value to this `Receiver`).
    pub fn try_recv(&mut self) -> Result<(T, Sender<T>), RecvError> {
        let shared = self.shared();
        let status = shared.status.load(Ordering::Relaxed);

        if status & SENDER_ALIVE != 0 {
            // The sender is still connected, thus hasn't send a value yet.
            return Err(RecvError::NoValue);
        } else if status & FILLED == 0 {
            // Sender disconnected and no value was send.
            return Err(RecvError::Disconnected);
        }

        // Reset the status.
        // Safety: since the `Sender` has been dropped we have unique access to
        // `shared` making Relaxed ordering fine.
        shared.status.store(
            RECEIVER_ALIVE | SENDER_ALIVE | SENDER_ACCESS | EMPTY,
            Ordering::Relaxed,
        );

        let msg = unsafe { (&*shared.message.get()).assume_init_read() };
        let sender = Sender {
            shared: self.shared,
        };
        Ok((msg, sender))
    }

    /// Returns a future that receives a value from the channel, waiting if the
    /// channel is empty.
    ///
    /// If the returned [`Future`] returns `None` it means the [`Sender`] is
    /// [disconnected] without sending a value. This is the same error as
    /// [`RecvError::Disconnected`]. [`RecvError::NoValue`] will never be
    /// returned, the `Future` will return [`Poll::Pending`] instead.
    ///
    /// [disconnected]: Receiver::is_connected
    pub fn recv<'r>(&'r mut self) -> RecvValue<'r, T> {
        RecvValue { receiver: self }
    }

    /// Returns an owned version of [`Receiver::recv`] that can only be used
    /// once.
    ///
    /// See [`Receiver::recv`] for more information.
    pub fn recv_once(self) -> RecvOnce<T> {
        RecvOnce { receiver: self }
    }

    /// Attempt to reset the channel.
    ///
    /// If the sender is disconnected this will return a new `Sender`. If the
    /// sender is still connected this will return `None`.
    ///
    /// # Notes
    ///
    /// If the channel contains a value it will be dropped.
    pub fn try_reset(&mut self) -> Option<Sender<T>> {
        match self.try_recv() {
            Ok((.., new_sender)) => Some(new_sender),
            Err(RecvError::NoValue) => None, // Sender still connected.
            Err(RecvError::Disconnected) => {
                // Sender has disconnected without sending a value so we can
                // safely reset the status, without having to worry about
                // dropping the value.
                // Safety: since the `Sender` has been dropped we have unique access
                // to `shared` making Relaxed ordering fine.
                self.shared().status.store(
                    RECEIVER_ALIVE | SENDER_ALIVE | SENDER_ACCESS | EMPTY,
                    Ordering::Relaxed,
                );
                Some(Sender {
                    shared: self.shared,
                })
            }
        }
    }

    /// Returns `true` if the `Sender` is connected.
    pub fn is_connected(&self) -> bool {
        // Relaxed is fine here since there is always a bit of a race condition
        // when using the method (and then doing something based on it).
        self.shared().status.load(Ordering::Relaxed) & SENDER_ALIVE != 0
    }

    /// Set the receiver waker to `waker`, if they are different.
    ///
    /// Returns `true` if the waker is changed, `false` otherwise.
    fn set_waker(&mut self, waker: &task::Waker) -> bool {
        let shared = self.shared();
        let mut receiver_waker = shared.receiver_waker.lock();

        if let Some(receiver_waker) = &*receiver_waker {
            if receiver_waker.will_wake(waker) {
                return false;
            }
        }

        *receiver_waker = Some(waker.clone());
        drop(receiver_waker);

        true
    }

    /// Reference the shared data.
    fn shared(&self) -> &Shared<T> {
        // Safety: see `shared` field.
        unsafe { self.shared.as_ref() }
    }
}

impl<T> fmt::Debug for Receiver<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Receiver")
    }
}

unsafe impl<T: Send> Send for Receiver<T> {}
unsafe impl<T: Send> Sync for Receiver<T> {}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        // Mark ourselves as dropped.
        let shared = self.shared();
        let old_status = shared.status.fetch_and(!RECEIVER_ALIVE, Ordering::AcqRel);

        if old_status & SENDER_ACCESS != 0 {
            // Sender is still alive, no need to drop.
            return;
        }

        // Drop the shared memory.
        unsafe { drop(Box::from_raw(self.shared.as_ptr())) }
    }
}

/// [`Future`] implementation behind [`Receiver::recv`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct RecvValue<'r, T> {
    receiver: &'r mut Receiver<T>,
}

macro_rules! recv_future_impl {
    ($self: ident, $ctx: ident) => {
        match $self.receiver.try_recv() {
            Ok(ok) => Poll::Ready(Some(ok)),
            Err(RecvError::NoValue) => {
                // The sender hasn't send a value yet, we'll set the waker.
                if !$self.receiver.set_waker($ctx.waker()) {
                    // Waker already set.
                    return Poll::Pending;
                }

                // It could be the case that the sender send a value in the time
                // between we last checked and we actually marked ourselves as
                // needing a wake up, so we need to check again.
                match $self.receiver.try_recv() {
                    Ok(ok) => Poll::Ready(Some(ok)),
                    // The `Sender` will wake us when the message is send.
                    Err(RecvError::NoValue) => Poll::Pending,
                    Err(RecvError::Disconnected) => Poll::Ready(None),
                }
            }
            Err(RecvError::Disconnected) => Poll::Ready(None),
        }
    };
}

impl<'r, T> Future for RecvValue<'r, T> {
    type Output = Option<(T, Sender<T>)>;

    fn poll(mut self: Pin<&mut Self>, ctx: &mut task::Context) -> Poll<Self::Output> {
        recv_future_impl!(self, ctx)
    }
}

impl<'r, T> Unpin for RecvValue<'r, T> {}

/// [`Future`] implementation behind [`Receiver::recv_once`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct RecvOnce<T> {
    receiver: Receiver<T>,
}

impl<T> Future for RecvOnce<T> {
    type Output = Option<T>;

    fn poll(mut self: Pin<&mut Self>, ctx: &mut task::Context) -> Poll<Self::Output> {
        match recv_future_impl!(self, ctx) {
            Poll::Ready(Some((value, _))) => Poll::Ready(Some(value)),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<T> Unpin for RecvOnce<T> {}

/// Data shared between [`Sender`] and [`Receiver`].
struct Shared<T> {
    /// A merging of the status of `message` and the liveness of the sender and
    /// receiver.
    status: AtomicU8,
    /// The message that may, or may not, be initialised depending on `status`.
    message: UnsafeCell<MaybeUninit<T>>,
    /// Waker used to wake the receiving end.
    receiver_waker: Mutex<Option<task::Waker>>,
}

impl<T> Shared<T> {
    /// Create a new `Shared` structure.
    const fn new() -> Shared<T> {
        Shared {
            status: AtomicU8::new(RECEIVER_ALIVE | SENDER_ALIVE | SENDER_ACCESS | EMPTY),
            message: UnsafeCell::new(MaybeUninit::uninit()),
            receiver_waker: const_mutex(None),
        }
    }
}

impl<T> Drop for Shared<T> {
    fn drop(&mut self) {
        if self.status.load(Ordering::Relaxed) & FILLED != 0 {
            unsafe { ptr::drop_in_place((&mut *self.message.get()).as_mut_ptr()) }
        }
    }
}
