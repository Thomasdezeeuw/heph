//! Test the [`set_message_loss`] function.
//!
//! # Notes
//!
//! This function needs to be in it's own binary since `set_message_loss` is set
//! globally.

#![feature(noop_waker)]

use std::future::Future;
use std::pin::{pin, Pin};
use std::task::{self, Poll};

use heph::actor::{self, actor_fn, NoMessages};
use heph::future::ActorFuture;
use heph::supervisor::NoSupervisor;
use heph::test::set_message_loss;

async fn expect_1_messages(mut ctx: actor::Context<usize>) {
    let msg = ctx.receive_next().await.expect("missing first message");
    assert_eq!(msg, 123);

    match ctx.receive_next().await {
        Ok(msg) => panic!("unexpected message: {msg:?}"),
        Err(NoMessages) => return,
    }
}

#[test]
fn actor_ref_message_loss() {
    let (future, actor_ref) =
        ActorFuture::new(NoSupervisor, actor_fn(expect_1_messages), ()).unwrap();
    let mut future = pin!(future);
    assert_eq!(poll(future.as_mut()), None);

    // Should arrive.
    actor_ref.try_send(123_usize).unwrap();
    assert_eq!(poll(future.as_mut()), None);

    // After setting the message loss to 100% no messages should arrive.
    set_message_loss(100);
    actor_ref.try_send(456_usize).unwrap();
    actor_ref.try_send(789_usize).unwrap();
    assert_eq!(poll(future.as_mut()), None);

    drop(actor_ref);
    assert_eq!(poll(future.as_mut()), Some(()));
}

pub fn poll<Fut: Future>(fut: Pin<&mut Fut>) -> Option<Fut::Output> {
    let mut ctx = task::Context::from_waker(task::Waker::noop());
    match fut.poll(&mut ctx) {
        Poll::Ready(output) => Some(output),
        Poll::Pending => None,
    }
}
