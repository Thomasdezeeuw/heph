//! Tests using multiple threads.

#![feature(track_caller)]

use std::thread;
use std::time::Duration;

use futures_test::task::noop_waker;

use inbox::{new_small, RecvError, SendError};

#[macro_use]
mod util;

#[test]
fn send_single_value() {
    let (mut sender, mut receiver) = new_small::<usize>(noop_waker());

    start_threads!(
        {
            expect_send!(sender, 1);
        },
        {
            expect_recv!(receiver, 1);
        }
    );
}

#[test]
fn zero_sized_types() {
    let (mut sender, mut receiver) = new_small(noop_waker());

    start_threads!(
        {
            expect_send!(sender, ());
        },
        {
            expect_recv!(receiver, ());
        }
    );
}

#[test]
fn receive_no_sender() {
    let (sender, mut receiver) = new_small::<usize>(noop_waker());

    start_threads!(
        {
            drop(sender);
        },
        {
            r#loop! {
                match receiver.try_recv() {
                    Ok(..) => panic!("unexpected receive of value"),
                    Err(RecvError::Empty) => {} // Try again.
                    Err(RecvError::Disconnected) => break,
                }
            }
        }
    );
}

#[test]
fn send_no_receiver() {
    let (mut sender, receiver) = new_small::<usize>(noop_waker());

    start_threads!(
        {
            thread::sleep(Duration::from_millis(1));
            r#loop! {
                match sender.try_send(1) {
                    Ok(()) => {} // Try again.
                    Err(SendError::Full(..)) => panic!("too slow!"),
                    Err(SendError::Disconnected(..)) => break,
                }
            }
        },
        {
            drop(receiver);
        }
    );
}

#[test]
fn sender_is_connected() {
    let (sender, receiver) = new_small::<usize>(noop_waker());

    start_threads!(
        {
            r#loop! {
                if !sender.is_connected() {
                    break;
                }
            }
        },
        {
            drop(receiver);
        }
    );
}

#[test]
fn receiver_is_connected() {
    let (sender, receiver) = new_small::<usize>(noop_waker());

    start_threads!(
        {
            drop(sender);
        },
        {
            r#loop! {
                if !receiver.is_connected() {
                    break;
                }
            }
        }
    );
}
