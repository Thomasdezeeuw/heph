#![allow(dead_code)] // Not all tests use all functions/types.

use std::fmt;
use std::net::SocketAddr;
use std::task::Poll;
use std::thread::sleep;
use std::time::Duration;

/// Bind to any IPv4 port on localhost.
pub fn any_local_address() -> SocketAddr {
    "127.0.0.1:0".parse().unwrap()
}

/// Bind to any IPv6 port on localhost.
pub fn any_local_ipv6_address() -> SocketAddr {
    "[::1]:0".parse().unwrap()
}

#[track_caller]
pub fn expect_pending<T>(poll: Poll<T>)
where
    T: fmt::Debug,
{
    match poll {
        Poll::Pending => {} // Ok.
        Poll::Ready(value) => panic!("unexpected `Poll::Ready({:?})`", value),
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
