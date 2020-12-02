//! Test the [`set_message_loss`] function.
//!
//! # Notes
//!
//! This function needs to be in it's own binary since `set_message_loss` is set
//! globally.

#![feature(never_type)]
#![cfg(feature = "test")]

use std::pin::Pin;
use std::task::Poll;

use heph::actor::{self, NoMessages};
use heph::test::{init_local_actor, poll_actor, set_message_loss};

async fn expect_1_messages(mut ctx: actor::Context<usize>) -> Result<(), !> {
    let msg = ctx.receive_next().await.expect("missing first message");
    assert_eq!(msg, 123);

    match ctx.receive_next().await {
        Ok(msg) => panic!("unexpected message: {:?}", msg),
        Err(NoMessages) => Ok(()),
    }
}

#[test]
fn actor_ref_message_loss() {
    let actor = expect_1_messages as fn(_) -> _;
    let (actor, actor_ref) = init_local_actor(actor, ()).unwrap();
    let mut actor = Box::pin(actor);

    // Should arive.
    actor_ref.try_send(123usize).unwrap();

    // After setting the message loss to 100% no messages should arive.
    set_message_loss(100);
    actor_ref.try_send(456usize).unwrap();
    actor_ref.try_send(789usize).unwrap();

    drop(actor_ref);
    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
}
