#![allow(dead_code)] // FIXME: remove.

//! Channel that allows a single message to be send, designed for use in Remote
//! Procedure Calls (RPC).

use std::cell::UnsafeCell;
use std::fmt;
use std::mem::{size_of, MaybeUninit};
use std::ptr::{self, NonNull};
use std::sync::atomic::{AtomicU8, Ordering};

use crate::rt::Waker;

/// Create a new one-shot channel.
pub(crate) fn channel<T>(waker: Waker) -> (Sender<T>, Receiver<T>) {
    let shared = Box::into_raw_non_null(Box::new(Shared::new(waker)));
    (Sender { shared }, Receiver { shared })
}

/// Bit mask to mark the receiver as alive.
const RECEIVER_ALIVE: u8 = 1 << (size_of::<u8>() * 8 - 1);
/// Bit mask to mark the sender as alive.
const SENDER_ALIVE: u8 = 1 << (size_of::<u8>() * 8 - 2);

/// Status of the message in `Shared`.
const EMPTY: u8 = 0;
/// NOTE: must be `EMPTY + 1` for `Receiver::try_send`!
const FILLED: u8 = 1;

/// Data shared between [`Sender`] and [`Receiver`].
struct Shared<T> {
    /// A merging of the status of `message` and the liveness of the sender and
    /// receiver.
    status: AtomicU8,
    /// The message that may, or may not, be initialised depending on `status`.
    message: UnsafeCell<MaybeUninit<T>>,
    /// Waker used to wake the receiving end.
    waker: Waker,
}

impl<T> Shared<T> {
    /// Create a new `Shared` structure.
    const fn new(waker: Waker) -> Shared<T> {
        Shared {
            status: AtomicU8::new(RECEIVER_ALIVE | SENDER_ALIVE | EMPTY),
            message: UnsafeCell::new(MaybeUninit::uninit()),
            waker,
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

/// The receiving half of the [one-shot channel].
///
/// This half can only be owned and used by one thread.
///
/// [one-shot channel]: crate::oneshot::channel
pub(crate) struct Receiver<T> {
    shared: NonNull<Shared<T>>,
}

/// Error returned by [`Receiver::try_recv`] and [`Receiver::try_peek`].
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum RecvError {
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
    // Alternative API, in which the receiver can be reused:
    // fn try_recv(&mut self) -> Result<, RecvError>;

    /// Attempts to receive a value and reset the channel.
    ///
    /// If it succeeds it returns the value and resets the channel, returning a
    /// new [`Sender`] (which can send a value to this `Receiver`).
    pub(crate) fn try_recv(&mut self) -> Result<(T, Sender<T>), RecvError> {
        let shared = self.shared();
        let status = shared.status.load(Ordering::Relaxed);

        if status & SENDER_ALIVE != 0 {
            // The sender is still connected, thus hasn't send a value yet.
            return Err(RecvError::NoValue);
        }

        if status & FILLED == 0 {
            // Sender disconnected and no value was send.
            return Err(RecvError::Disconnected);
        }

        // Reset the status.
        shared
            .status
            .store(RECEIVER_ALIVE | SENDER_ALIVE | EMPTY, Ordering::Relaxed);

        let msg = unsafe { (&*shared.message.get()).read() };
        Ok((
            msg,
            Sender {
                shared: self.shared,
            },
        ))
    }

    /// Attempts to peek a value.
    pub(crate) fn try_peek(&self) -> Result<T, RecvError>
    where
        T: Clone,
    {
        let shared = self.shared();
        let status = shared.status.load(Ordering::Relaxed);

        if status & SENDER_ALIVE != 0 {
            // The sender is still connected, thus hasn't send a value yet.
            return Err(RecvError::NoValue);
        }

        if status & FILLED == 0 {
            // Sender disconnected and no value was send.
            return Err(RecvError::Disconnected);
        }

        unsafe { Ok(T::clone((&*shared.message.get()).get_ref())) }
    }

    /// Attempts to reset the channel, returning a new sender for the channel.
    ///
    /// Fails if the sender is still connected.
    pub(crate) fn try_reset(&mut self) -> Result<Sender<T>, ()> {
        let shared = self.shared();
        let status = shared.status.load(Ordering::Relaxed);
        if status & SENDER_ALIVE != 0 {
            // Sender is alive, can't reset.
            return Err(());
        }

        if status & !RECEIVER_ALIVE == FILLED {
            // Channel holds a value we need to drop.
            unsafe { ptr::drop_in_place((&mut *shared.message.get()).as_mut_ptr()) }
        }

        // Reset the status.
        shared
            .status
            .store(RECEIVER_ALIVE | SENDER_ALIVE | EMPTY, Ordering::Relaxed);

        Ok(Sender {
            shared: self.shared,
            // TODO: waker
        })
    }

    /// Returns true if the sender is connected.
    pub(crate) fn is_connected(&self) -> bool {
        // Relaxed is fine here since there is always a bit of a race condition
        // when using the method (and then doing something based on it).
        self.shared().status.load(Ordering::Relaxed) & SENDER_ALIVE != 0
    }

    /// Reference the shared data.
    fn shared(&self) -> &Shared<T> {
        // This is safe because it always points to valid memory.
        unsafe { self.shared.as_ref() }
    }
}

impl<T> fmt::Debug for Receiver<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Receiver")
    }
}

unsafe impl<T: Send> Send for Receiver<T> {}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        // Mark ourselves as dropped.
        let old_status = self
            .shared()
            .status
            .fetch_and(!RECEIVER_ALIVE, Ordering::Release);

        if old_status & !RECEIVER_ALIVE > FILLED {
            // Sender is still alive, no need to drop.
            return;
        }

        // Drop the shared memory.
        unsafe { drop(Box::from_raw(self.shared.as_ptr())) }
    }
}

/// The sending-half of the [one-shot channel].
///
/// This half can only be owned and used by one thread.
///
/// [one-shot channel]: crate::oneshot::channel
pub(crate) struct Sender<T> {
    shared: NonNull<Shared<T>>,
}

impl<T> Sender<T> {
    /// Attempts to send a `message` into the channel. If this returns an error
    /// it means the receiver has disconnected (has been dropped).
    pub(crate) fn try_send(self, message: T) -> Result<(), T> {
        if !self.is_connected() {
            return Err(message);
        }

        let shared = self.shared();

        // This is safe because we're the only sender.
        unsafe { ptr::write(shared.message.get(), MaybeUninit::new(message)) };

        // Mark the item as filled.
        let old_status = shared.status.fetch_add(1, Ordering::Relaxed);
        debug_assert_eq!(old_status, RECEIVER_ALIVE | SENDER_ALIVE | EMPTY);

        // Note: we wake in the `Drop` impl.
        Ok(())
    }

    /// Returns true if the receiver is connected.
    pub(crate) fn is_connected(&self) -> bool {
        // Relaxed is fine here since there is always a bit of a race condition
        // when using the method (and then doing something based on it).
        self.shared().status.load(Ordering::Relaxed) & RECEIVER_ALIVE != 0
    }

    /// Reference the shared data.
    fn shared(&self) -> &Shared<T> {
        // This is safe because it always points to valid memory.
        unsafe { self.shared.as_ref() }
    }
}

impl<T> fmt::Debug for Sender<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Sender")
    }
}

unsafe impl<T: Send> Send for Sender<T> {}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        // Mark ourselves as dropped.
        let old_status = self
            .shared()
            .status
            .fetch_and(!SENDER_ALIVE, Ordering::Release);

        if old_status & !SENDER_ALIVE > FILLED {
            // Receiver is still alive, no need to drop. But we do need to wake
            // the other wise.
            self.shared().waker.wake();
            return;
        }

        // Drop the shared memory.
        unsafe { drop(Box::from_raw(self.shared.as_ptr())) }
    }
}
