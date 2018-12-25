//! One shot channels.
//!
//! # Request-response pattern
//!
//! Even though heph uses an event driven programming model sometimes the
//! easiest solution is to use a request-response pattern, wherein one actor
//! sends a request to another actor and awaits (in non-blocking way) the
//! response. One shot channels are perfectly suited for this pattern, as the
//! example below demonstrates.
//!
//! ```
//! #![feature(async_await, await_macro, futures_api, never_type)]
//!
//! use heph::actor::{ActorContext, actor_factory};
//! use heph::actor_ref::LocalActorRef;
//! use heph::channel::oneshot;
//!
//! /// Type representing a database connection.
//! #[derive(Debug)]
//! struct Conn;
//!
//! impl Conn {
//!     /// Setup a new database connection.
//!     fn new() -> Conn { Conn }
//! }
//!
//! /// Message type for database connection pool actor.
//! enum ConnMsg {
//!     /// Request a connection to database, which will be send across the one
//!     /// shot channel as response.
//!     Get(oneshot::Sender<Conn>),
//! }
//!
//! /// Actor that manages a pool of database connections.
//! async fn db_conn_actor(mut ctx: ActorContext<ConnMsg>, mut pool: Vec<Conn>) -> Result<(), !> {
//!     loop {
//!         match await!(ctx.receive()) {
//!             ConnMsg::Get(sender) => {
//!                 let conn = pool.pop().unwrap_or(Conn::new());
//!                 if let Err(err) = sender.send(conn) {
//!                     // Receiving actor is no longer interested. So we can
//!                     // put the connection back into the pool.
//!                     pool.push(err.0);
//!                 }
//!             },
//!             _ => { /* Do more stuff like adding back the connections. */ },
//!         }
//!     }
//! }
//!
//! /// Our actor that uses a database connection.
//! async fn actor(mut ctx: ActorContext<()>, mut db_actor: LocalActorRef<ConnMsg>) -> Result<(), ()> {
//!     // Create a new one shot channel.
//!     let (sender, receiver) = oneshot::channel();
//!     // Send a get request to the database connection actor.
//!     db_actor.send(ConnMsg::Get(sender)).map_err(|_| ())?;
//!
//!     // Wait for the connection to be returned.
//!     let conn = await!(receiver);
//!
//!     // Use the database connection here.
//! #   drop(conn);
//!     Ok(())
//! }
//! ```

use std::future::Future;
use std::pin::{Pin, Unpin};
use std::task::{Poll, LocalWaker};

use crate::channel::{NoReceiver, NoValue};
use crate::util::Shared;

/// Sending half of the one shot channel.
#[derive(Debug)]
pub struct Sender<T> {
    inner: Shared<ChannelInner<T>>,
}

impl<T> Sender<T> {
    /// Attempts to send a value across the channel.
    ///
    /// If the receiving half of the channel was dropped, before a value could
    /// be send across the channel, a `NoReceiver` error is returned.
    pub fn send(mut self, value: T) -> Result<(), NoReceiver<T>> {
        if self.inner.strong_count() == 1 {
            return Err(NoReceiver(value));
        }

        let mut inner = self.inner.borrow_mut();
        inner.value = Some(value);
        if let Some(ref waker) = inner.waker {
            waker.wake();
        }
        Ok(())
    }
}

/// Receiving half of the one shot channel.
#[derive(Debug)]
pub struct Receiver<T> {
    inner: Shared<ChannelInner<T>>,
}

impl<T: Unpin> Future for Receiver<T> {
    type Output = Result<T, NoValue>;

    fn poll(self: Pin<&mut Self>, lw: &LocalWaker) -> Poll<Self::Output> {
        let this = Pin::get_mut(self);
        if let Some(value) = this.inner.borrow_mut().value.take() {
            return Poll::Ready(Ok(value));
        }

        if this.inner.strong_count() == 1 {
            Poll::Ready(Err(NoValue))
        } else {
            this.inner.borrow_mut().waker = Some(lw.clone());
            Poll::Pending
        }
    }
}

/// Inside of the one shot channel, shared by 1 `Sender` and 1 `Receiver`.
#[derive(Debug)]
struct ChannelInner<T> {
    /// Value set by `Sender.send` and taken by `Receiver.poll`.
    value: Option<T>,
    /// Waker possibly set by calling `Receiver.poll` and awoken by
    /// `Sender.send`, if set.
    waker: Option<LocalWaker>,
}

/// Creates a new asynchronous one shot channel, returning the sending and
/// receiving halves.
pub fn channel<T: Unpin>() -> (Sender<T>, Receiver<T>) {
    let shared = Shared::new(ChannelInner { value: None, waker: None });
    (Sender { inner: shared.clone() }, Receiver { inner: shared })
}

#[cfg(all(test, feature = "test"))]
mod tests {
    use std::future::Future;
    use std::task::Poll;

    use crate::channel::{NoReceiver, NoValue, oneshot};
    use crate::test::new_count_waker;

    #[test]
    fn sending_wakes_waker() {
        let (sender, receiver) = oneshot();
        let mut receiver = Box::pinned(receiver);
        let (waker, count) = new_count_waker();

        assert_eq!(count.get(), 0);
        assert_eq!(receiver.as_mut().poll(&waker), Poll::Pending);
        assert_eq!(count.get(), 0);

        sender.send(()).unwrap();
        assert_eq!(count.get(), 1);
        assert_eq!(receiver.as_mut().poll(&waker), Poll::Ready(Ok(())));
    }

    #[test]
    fn sending_ok_with_no_waker() {
        let (sender, receiver) = oneshot();
        let mut receiver = Box::pinned(receiver);
        let (waker, count) = new_count_waker();

        assert_eq!(count.get(), 0);
        sender.send(()).unwrap();
        assert_eq!(count.get(), 0);
        assert_eq!(receiver.as_mut().poll(&waker), Poll::Ready(Ok(())));
    }

    #[test]
    fn no_receiver() {
        let (sender, receiver) = oneshot();
        drop(receiver);
        assert_eq!(sender.send(()), Err(NoReceiver(())));
    }

    #[test]
    fn no_sender() {
        let (sender, receiver) = oneshot::<()>();
        let mut receiver = Box::pinned(receiver);
        let (waker, count) = new_count_waker();

        drop(sender);
        assert_eq!(count.get(), 0);
        assert_eq!(receiver.as_mut().poll(&waker), Poll::Ready(Err(NoValue)));
    }
}
