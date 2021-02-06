#![allow(dead_code)] // Not all tests use all functions/types.
#![cfg(feature = "test")]

use std::fmt;
use std::future::Future;
use std::mem::size_of;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{self, Poll};
use std::thread::sleep;
use std::time::Duration;

use heph::test::poll_actor;
use heph::Actor;

#[track_caller]
macro_rules! limited_loop {
    ($($arg: tt)*) => {{
        let mut range = (0..1_000);
        while range.next().is_some() {
            $($arg)*
        }

        if range.is_empty() {
            panic!("looped too many iterations");
        }
    }}
}

pub fn assert_send<T: Send>() {}

pub fn assert_sync<T: Sync>() {}

#[track_caller]
pub fn assert_size<T>(expected: usize) {
    assert_eq!(size_of::<T>(), expected);
}

/// Bind to any IPv4 port on localhost.
pub fn any_local_address() -> SocketAddr {
    "127.0.0.1:0".parse().unwrap()
}

/// Bind to any IPv6 port on localhost.
pub fn any_local_ipv6_address() -> SocketAddr {
    "[::1]:0".parse().unwrap()
}

/// Returns an address to which the connection will be refused.
pub fn refused_address() -> SocketAddr {
    "127.0.0.1:65535".parse().unwrap()
}

/// Stage of a test actor.
///
/// When testing `Future`s it sometimes hard to tell which futures have and
/// haven't been completed. If a test depends on the fact that a certrain future
/// inside an actor has been completed, but another has not this can be help to
/// determine that.
///
/// For example, we want to test that a `TcpStream` is connected, but can't read
/// anything yet (because we haven't written anything), before writing to the
/// connection and testing that it can be read. This depends on the fact that
/// the actor will return pending the moment it can't read anything, with
/// `Stage` we know the actor is connected but hasn't read anything.
#[derive(Debug)]
pub struct Stage(AtomicUsize);

impl Stage {
    pub const fn new() -> Stage {
        Stage(AtomicUsize::new(0))
    }

    /// Polls `actor` until `stage` is at `wanted` stage.
    #[track_caller]
    pub fn poll_till<A>(&self, mut actor: Pin<&mut A>, want: usize) -> Poll<Result<(), A::Error>>
    where
        A: Actor,
    {
        let mut n = 0;
        loop {
            let res = poll_actor(actor.as_mut());
            let state = self.0.load(Ordering::SeqCst);
            if res.is_ready() || state >= want {
                assert_eq!(state, want, "unexpected state");
                return res;
            }

            if n > 100 {
                panic!("looped too many times");
            }
            n += 1;

            // Don't want to busy loop.
            sleep(Duration::from_millis(1));
        }
    }

    /// Updates the stage to `stage`.
    pub fn update(&self, stage: usize) {
        self.0.store(stage, Ordering::SeqCst)
    }
}

#[track_caller]
pub fn expect_pending<T>(poll: Poll<T>)
where
    T: fmt::Debug,
{
    match poll {
        Poll::Pending => {} // Ok.
        Poll::Ready(value) => panic!("expected pending, got `Poll::Ready({:?})`", value),
    }
}

#[track_caller]
pub fn expect_ready_ok<T, E>(poll: Poll<Result<T, E>>, expected: T)
where
    T: fmt::Debug + PartialEq,
    E: fmt::Display,
{
    match poll {
        Poll::Pending => panic!("unexpected `Poll::Pending`"),
        Poll::Ready(Ok(value)) => assert_eq!(value, expected),
        Poll::Ready(Err(err)) => panic!("unexpected err: {}", err),
    }
}

/// Call `f` in a loop until it returns `Poll::Ready(T)`.
#[track_caller]
pub fn loop_expect_ready_ok<F, T, E>(mut f: F, expected: T)
where
    F: FnMut() -> Poll<Result<T, E>>,
    T: fmt::Debug + PartialEq,
    E: fmt::Display,
{
    loop {
        match f() {
            Poll::Pending => {}
            Poll::Ready(Ok(value)) => {
                assert_eq!(value, expected);
                return;
            }
            Poll::Ready(Err(err)) => panic!("unexpected err: {}", err),
        }

        // Don't want to busy loop.
        sleep(Duration::from_millis(10));
    }
}

/// Runs all `actors`.
pub fn run_actors(mut actors: Vec<Pin<Box<dyn Actor<Error = !>>>>) {
    for _ in 0..20 {
        if actors.is_empty() {
            return;
        }

        actors.drain_filter(|actor| match poll_actor(Pin::as_mut(actor)) {
            Poll::Pending => false,
            Poll::Ready(Ok(())) => true,
            Poll::Ready(Err(_)) => unreachable!(),
        });
        sleep(Duration::from_millis(10));
    }

    assert!(actors.is_empty(), "not all actors have completed");
}

/// Returns a [`Future`] that return [`Poll::Pending`] once, without waking
/// itself.
pub const fn pending_once() -> PendingOnce {
    PendingOnce(false)
}

pub struct PendingOnce(bool);

impl Future for PendingOnce {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        if self.0 {
            Poll::Ready(())
        } else {
            self.0 = true;
            Poll::Pending
        }
    }
}
