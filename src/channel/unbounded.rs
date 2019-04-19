//! Unbounded channels.

use std::collections::VecDeque;
use std::future::Future;
use std::marker::Unpin;
use std::pin::Pin;
use std::task::{self, Poll, Waker};

use futures_core::stream::Stream;

use crate::channel::{NoReceiver, NoValue};
use crate::util::Shared;

/// Sending half of the unbounded channel.
#[derive(Debug)]
pub struct Sender<T> {
    inner: Shared<ChannelInner<T>>,
}

impl<T> Sender<T> {
    /// Attempts to send a value across the channel.
    ///
    /// If the receiving half of the channel was dropped an error is returned.
    pub fn send(&mut self, value: T) -> Result<(), NoReceiver<T>> {
        if !self.is_connected() {
            return Err(NoReceiver(value));
        }

        let mut inner = self.inner.borrow_mut();
        inner.values.push_back(value);
        if let Some(ref waker) = inner.waker {
            waker.wake_by_ref();
        }
        Ok(())
    }

    /// Whether or not the receiving half is still connected.
    pub fn is_connected(&self) -> bool {
        self.inner.strong_count() >= 2
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        if let Some(ref waker) = self.inner.borrow().waker {
            waker.wake_by_ref();
        }
    }
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

    fn poll_next(self: Pin<&mut Self>, ctx: &mut task::Context) -> Poll<Option<Self::Item>> {
        let this = Pin::get_mut(self);
        match this.try_receive() {
            Ok(Some(value)) => Poll::Ready(Some(value)),
            Ok(None) => {
                this.inner.borrow_mut().waker = Some(ctx.waker().clone());
                Poll::Pending
            },
            Err(_) => Poll::Ready(None),
        }
    }
}

/// Future that receives a single value from [`Receiver`].
///
/// See [`Receiver::receive_one`].
#[derive(Debug)]
pub struct ReceiveOne<'r, T> {
    inner: &'r mut Receiver<T>,
}

impl<'r, T: Unpin> Future for ReceiveOne<'r, T> {
    type Output = Result<T, NoValue>;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context) -> Poll<Self::Output> {
        let this = Pin::get_mut(self);
        match this.inner.try_receive() {
            Ok(Some(value)) => Poll::Ready(Ok(value)),
            Ok(None) => {
                this.inner.inner.borrow_mut().waker = Some(ctx.waker().clone());
                Poll::Pending
            },
            Err(err) => Poll::Ready(Err(err)),
        }
    }
}

#[derive(Debug)]
struct ChannelInner<T> {
    /// Values added by `Sender.send` and removed by `Receiver.try_receive`.
    values: VecDeque<T>,
    /// Waker set by calling `Receiver.poll_next` or `ReceiveOne.poll` and
    /// awoken by `Sender.send`, if set.
    waker: Option<Waker>,
}

/// Creates a new asynchronous unbounded channel, returning the sending and
/// receiving halves.
pub fn channel<T: Unpin>() -> (Sender<T>, Receiver<T>) {
    let shared = Shared::new(ChannelInner { values: VecDeque::new(), waker: None });
    (Sender { inner: shared.clone() }, Receiver { inner: shared })
}

#[cfg(all(test, feature = "test"))]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{self, Poll};

    use futures_core::stream::Stream;
    use futures_test::task::new_count_waker;

    use crate::channel::{unbounded, NoReceiver, NoValue};

    #[test]
    fn sending_wakes_receiver() {
        let (mut sender, receiver) = unbounded();
        let mut receiver = Box::pin(receiver);
        let (waker, count) = new_count_waker();
        let mut ctx = task::Context::from_waker(&waker);

        assert_eq!(count, 0);
        assert_eq!(receiver.as_mut().poll_next(&mut ctx), Poll::Pending);
        assert_eq!(count, 0);

        sender.send(()).unwrap();
        assert_eq!(count, 1);
        assert_eq!(receiver.as_mut().poll_next(&mut ctx), Poll::Ready(Some(())));
    }

    #[test]
    fn sending_ok_with_no_waker() {
        let (mut sender, receiver) = unbounded();
        let mut receiver = Box::pin(receiver);
        let (waker, count) = new_count_waker();
        let mut ctx = task::Context::from_waker(&waker);

        assert_eq!(count, 0);
        sender.send(()).unwrap();
        assert_eq!(count, 0);
        assert_eq!(receiver.as_mut().poll_next(&mut ctx), Poll::Ready(Some(())));
    }

    #[test]
    fn wake_when_sender_is_dropped() {
        let (sender, receiver) = unbounded::<()>();
        let mut receiver = Box::pin(receiver);
        let (waker, count) = new_count_waker();
        let mut ctx = task::Context::from_waker(&waker);

        assert_eq!(count, 0);
        assert_eq!(receiver.as_mut().poll_next(&mut ctx), Poll::Pending);
        assert_eq!(count, 0);

        drop(sender);
        assert_eq!(count, 1);
        assert_eq!(receiver.as_mut().poll_next(&mut ctx), Poll::Ready(None));
    }

    #[test]
    fn receive_stream_after_sender_drop() {
        let (mut sender, receiver) = unbounded();
        let mut receiver = Box::pin(receiver);
        let (waker, count) = new_count_waker();
        let mut ctx = task::Context::from_waker(&waker);

        assert_eq!(count, 0);
        sender.send(1).unwrap();
        sender.send(2).unwrap();
        drop(sender);

        assert_eq!(count, 0);
        assert_eq!(receiver.as_mut().poll_next(&mut ctx), Poll::Ready(Some(1)));
        assert_eq!(receiver.as_mut().poll_next(&mut ctx), Poll::Ready(Some(2)));
        assert_eq!(receiver.as_mut().poll_next(&mut ctx), Poll::Ready(None));
    }

    #[test]
    fn receive_one() {
        let (mut sender, mut receiver) = unbounded();
        let (waker, count) = new_count_waker();
        let mut ctx = task::Context::from_waker(&waker);

        assert_eq!(count, 0);
        sender.send(1).unwrap();

        assert_eq!(count, 0);
        assert_eq!(Pin::new(&mut receiver.receive_one()).as_mut().poll(&mut ctx), Poll::Ready(Ok(1)));
        assert_eq!(Pin::new(&mut receiver.receive_one()).as_mut().poll(&mut ctx), Poll::Pending);

        sender.send(2).unwrap();
        assert_eq!(count, 1);
        drop(sender);
        assert_eq!(count, 2);

        assert_eq!(Pin::new(&mut receiver.receive_one()).as_mut().poll(&mut ctx), Poll::Ready(Ok(2)));
        assert_eq!(Pin::new(&mut receiver.receive_one()).as_mut().poll(&mut ctx), Poll::Ready(Err(NoValue)));
    }

    #[test]
    fn receive_one_after_sender_drop() {
        let (mut sender, mut receiver) = unbounded();
        let (waker, count) = new_count_waker();
        let mut ctx = task::Context::from_waker(&waker);

        assert_eq!(count, 0);
        sender.send(1).unwrap();
        sender.send(2).unwrap();
        drop(sender);

        assert_eq!(count, 0);
        assert_eq!(Pin::new(&mut receiver.receive_one()).as_mut().poll(&mut ctx), Poll::Ready(Ok(1)));
        assert_eq!(Pin::new(&mut receiver.receive_one()).as_mut().poll(&mut ctx), Poll::Ready(Ok(2)));
        assert_eq!(Pin::new(&mut receiver.receive_one()).as_mut().poll(&mut ctx), Poll::Ready(Err(NoValue)));
    }

    #[test]
    fn no_receiver() {
        let (mut sender, receiver) = unbounded();
        drop(receiver);
        assert_eq!(sender.send(()), Err(NoReceiver(())));
    }

    #[test]
    fn no_sender() {
        let (sender, receiver) = unbounded::<()>();
        let mut receiver = Box::pin(receiver);
        let (waker, count) = new_count_waker();
        let mut ctx = task::Context::from_waker(&waker);

        drop(sender);
        assert_eq!(count, 0);
        assert_eq!(receiver.as_mut().poll_next(&mut ctx), Poll::Ready(None));
    }
}
