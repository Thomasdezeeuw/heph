//! Bounded channels.

// TODO: Wake sender if channel has space for more values? This likely makes
// more sense if `Sender` would implement `Sink`.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::{Pin, Unpin};
use std::task::{Poll, LocalWaker};

use futures_core::stream::Stream;

use crate::channel::NoValue;
use crate::util::Shared;

/// Sending half of the bounded channel.
#[derive(Debug)]
pub struct Sender<T> {
    inner: Shared<ChannelInner<T>>,
}

impl<T> Sender<T> {
    /// Attempts to send a value across the channel.
    ///
    /// If the receiving half of the channel was dropped or if the channel is
    /// full an error is returned.
    pub fn send(&mut self, value: T) -> Result<(), SendError<T>> {
        if self.inner.strong_count() == 1 {
            return Err(SendError::NoReceiver(value));
        }

        let mut inner = self.inner.borrow_mut();
        if !inner.has_capacity() {
            return Err(SendError::NoCapacity(value));
        }

        inner.values.push_back(value);
        if let Some(ref waker) = inner.waker {
            waker.wake();
        }
        Ok(())
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        if let Some(ref waker) = self.inner.borrow_mut().waker {
            waker.wake();
        }
    }
}

/// Error returned when sending a value.
///
/// The send value can be retrieved.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SendError<T> {
    /// Receiving half of the channel was dropped.
    NoReceiver(T),
    /// The channel has no capacity left.
    NoCapacity(T),
}

/// Receiving half of the bounded channel.
#[derive(Debug)]
pub struct Receiver<T> {
    inner: Shared<ChannelInner<T>>,
}

impl<T> Receiver<T> {
    /// Returns a [`Future`] that returns a single value, if any is send.
    ///
    /// The `Future`'s lifetime is bound to the receiver.
    pub fn receive_one<'r>(&'r mut self) -> ReceiveOne<'r, T> {
        ReceiveOne { inner: self }
    }

    /// Try to take a single value from the channel.
    fn try_receive(&mut self) -> Result<Option<T>, NoValue> {
        if let Some(value) = self.inner.borrow_mut().values.pop_front() {
            return Ok(Some(value));
        }

        if self.inner.strong_count() == 1 {
            Err(NoValue)
        } else {
            Ok(None)
        }
    }
}

impl<T: Unpin> Stream for Receiver<T> {
    type Item = T;

    fn poll_next(self: Pin<&mut Self>, lw: &LocalWaker) -> Poll<Option<Self::Item>> {
        let this = Pin::get_mut(self);
        match this.try_receive() {
            Ok(Some(value)) => Poll::Ready(Some(value)),
            Ok(None) => {
                this.inner.borrow_mut().waker = Some(lw.clone());
                Poll::Pending
            },
            Err(_) => Poll::Ready(None),
        }
    }
}

/// Future that receives a single value from [`Receiver`].
///
/// See [`Receiver.receive_one`].
///
/// [`Receiver`]: struct.Receiver.html
/// [`Receiver.receive_one`]: struct.Receiver.html#method.receive_one
#[derive(Debug)]
pub struct ReceiveOne<'r, T> {
    inner: &'r mut Receiver<T>,
}

impl<'r, T: Unpin> Future for ReceiveOne<'r, T> {
    type Output = Result<T, NoValue>;

    fn poll(self: Pin<&mut Self>, lw: &LocalWaker) -> Poll<Self::Output> {
        let this = Pin::get_mut(self);
        match this.inner.try_receive() {
            Ok(Some(value)) => Poll::Ready(Ok(value)),
            Ok(None) => {
                this.inner.inner.borrow_mut().waker = Some(lw.clone());
                Poll::Pending
            },
            Err(err) => Poll::Ready(Err(err))
        }
    }
}

/// Inside of the bounded channel, shared by 1 `Sender` and 1 `Receiver`.
#[derive(Debug)]
struct ChannelInner<T> {
    /// Values added by `Sender.send` and removed by `Receiver.try_receive`.
    values: VecDeque<T>,
    /// Waker set by calling `Receiver.poll_next` or `ReceiveOne.poll` and
    /// awoken by `Sender.send`, if set.
    waker: Option<LocalWaker>,
}

impl<T> ChannelInner<T> {
    /// Whether or not the channel has capacity for one more value.
    fn has_capacity(&self) -> bool {
        self.values.capacity() > self.values.len()
    }
}

/// Creates a new asynchronous bounded channel, returning the sending and
/// receiving halves.
pub fn channel<T: Unpin>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    debug_assert!(capacity >= 1, "can't create a bounded channel with 0 capacity");
    let shared = Shared::new(ChannelInner {
        values: VecDeque::with_capacity(capacity),
        waker: None
    });
    (Sender { inner: shared.clone() }, Receiver { inner: shared })
}

#[cfg(all(test, feature = "test"))]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::task::Poll;

    use futures_core::stream::Stream;

    use crate::channel::bounded::SendError;
    use crate::channel::{bounded, NoValue};
    use crate::test::new_count_waker;

    #[test]
    fn sending_wakes_receiver() {
        let (mut sender, receiver) = bounded(1);
        let mut receiver = Box::pinned(receiver);
        let (waker, count) = new_count_waker();

        assert_eq!(count.get(), 0);
        assert_eq!(receiver.as_mut().poll_next(&waker), Poll::Pending);
        assert_eq!(count.get(), 0);

        sender.send(()).unwrap();
        assert_eq!(count.get(), 1);
        assert_eq!(receiver.as_mut().poll_next(&waker), Poll::Ready(Some(())));
    }

    #[test]
    fn sending_ok_with_no_waker() {
        let (mut sender, receiver) = bounded(1);
        let mut receiver = Box::pinned(receiver);
        let (waker, count) = new_count_waker();

        assert_eq!(count.get(), 0);
        sender.send(()).unwrap();
        assert_eq!(count.get(), 0);
        assert_eq!(receiver.as_mut().poll_next(&waker), Poll::Ready(Some(())));
    }

    #[test]
    fn wake_when_sender_is_dropped() {
        let (sender, receiver) = bounded::<()>(1);
        let mut receiver = Box::pinned(receiver);
        let (waker, count) = new_count_waker();

        assert_eq!(count.get(), 0);
        assert_eq!(receiver.as_mut().poll_next(&waker), Poll::Pending);
        assert_eq!(count.get(), 0);

        drop(sender);
        assert_eq!(count.get(), 1);
        assert_eq!(receiver.as_mut().poll_next(&waker), Poll::Ready(None));
    }

    #[test]
    fn receive_stream_after_sender_drop() {
        let (mut sender, receiver) = bounded(2);
        let mut receiver = Box::pinned(receiver);
        let (waker, count) = new_count_waker();

        assert_eq!(count.get(), 0);
        sender.send(1).unwrap();
        sender.send(2).unwrap();
        drop(sender);

        assert_eq!(count.get(), 0);
        assert_eq!(receiver.as_mut().poll_next(&waker), Poll::Ready(Some(1)));
        assert_eq!(receiver.as_mut().poll_next(&waker), Poll::Ready(Some(2)));
        assert_eq!(receiver.as_mut().poll_next(&waker), Poll::Ready(None));
    }

    #[test]
    fn receive_one() {
        let (mut sender, mut receiver) = bounded(2);
        let (waker, count) = new_count_waker();

        assert_eq!(count.get(), 0);
        sender.send(1).unwrap();

        assert_eq!(count.get(), 0);
        assert_eq!(Pin::new(&mut receiver.receive_one()).as_mut().poll(&waker), Poll::Ready(Ok(1)));
        assert_eq!(Pin::new(&mut receiver.receive_one()).as_mut().poll(&waker), Poll::Pending);

        sender.send(2).unwrap();
        assert_eq!(count.get(), 1);
        drop(sender);
        assert_eq!(count.get(), 2);

        assert_eq!(Pin::new(&mut receiver.receive_one()).as_mut().poll(&waker), Poll::Ready(Ok(2)));
        assert_eq!(Pin::new(&mut receiver.receive_one()).as_mut().poll(&waker), Poll::Ready(Err(NoValue)));
    }

    #[test]
    fn receive_one_after_sender_drop() {
        let (mut sender, mut receiver) = bounded(2);
        let (waker, count) = new_count_waker();

        assert_eq!(count.get(), 0);
        sender.send(1).unwrap();
        sender.send(2).unwrap();
        drop(sender);

        assert_eq!(count.get(), 0);
        assert_eq!(Pin::new(&mut receiver.receive_one()).as_mut().poll(&waker), Poll::Ready(Ok(1)));
        assert_eq!(Pin::new(&mut receiver.receive_one()).as_mut().poll(&waker), Poll::Ready(Ok(2)));
        assert_eq!(Pin::new(&mut receiver.receive_one()).as_mut().poll(&waker), Poll::Ready(Err(NoValue)));
    }

    #[test]
    fn sending_no_capacity() {
        let (mut sender, receiver) = bounded(1);
        let mut receiver = Box::pinned(receiver);
        let (waker, count) = new_count_waker();

        assert_eq!(count.get(), 0);
        sender.send(1).unwrap();
        assert_eq!(sender.send(2), Err(SendError::NoCapacity(2)));

        assert_eq!(count.get(), 0);
        assert_eq!(receiver.as_mut().poll_next(&waker), Poll::Ready(Some(1)));
        assert_eq!(receiver.as_mut().poll_next(&waker), Poll::Pending);

        sender.send(2).unwrap();
        assert_eq!(count.get(), 1);
        assert_eq!(receiver.as_mut().poll_next(&waker), Poll::Ready(Some(2)));
        assert_eq!(receiver.as_mut().poll_next(&waker), Poll::Pending);

        drop(sender);
        assert_eq!(count.get(), 2);
        assert_eq!(receiver.as_mut().poll_next(&waker), Poll::Ready(None));
    }

    #[test]
    fn no_receiver() {
        let (mut sender, receiver) = bounded(1);
        drop(receiver);
        assert_eq!(sender.send(()), Err(SendError::NoReceiver(())));
    }

    #[test]
    fn no_sender() {
        let (sender, receiver) = bounded::<()>(1);
        let mut receiver = Box::pinned(receiver);
        let (waker, count) = new_count_waker();

        drop(sender);
        assert_eq!(count.get(), 0);
        assert_eq!(receiver.as_mut().poll_next(&waker), Poll::Ready(None));
    }
}
