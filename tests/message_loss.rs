//! Test the [`set_message_loss`] function.
//!
//! # Notes
//!
//! This function needs to be in it's own binary since `set_message_loss` is set
//! globally.

use std::pin::Pin;
use std::task::Poll;

use heph::actor::{self, actor_fn, NoMessages};
use heph::test::set_message_loss;
use heph_rt::test::{init_local_actor, poll_actor};
use heph_rt::ThreadLocal;

async fn expect_1_messages(mut ctx: actor::Context<usize, ThreadLocal>) {
    let msg = ctx.receive_next().await.expect("missing first message");
    assert_eq!(msg, 123);

    match ctx.receive_next().await {
        Ok(msg) => panic!("unexpected message: {msg:?}"),
        Err(NoMessages) => return,
    }
}

#[test]
fn actor_ref_message_loss() {
    let (actor, actor_ref) = init_local_actor(actor_fn(expect_1_messages), ()).unwrap();
    let mut actor = Box::pin(actor);

    // Should arrive.
    actor_ref.try_send(123_usize).unwrap();

    // After setting the message loss to 100% no messages should arrive.
    set_message_loss(100);
    actor_ref.try_send(456_usize).unwrap();
    actor_ref.try_send(789_usize).unwrap();

    drop(actor_ref);
    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
}
