//! Tests for the `actor::Context`.

#![cfg(feature = "test")]

use std::pin::Pin;
use std::task::Poll;

use heph::actor::{self, context, NoMessages, RecvError};
use heph::test::{init_local_actor, poll_actor};

use crate::util::{assert_send, assert_sync};

#[test]
fn thread_safe_is_send_sync() {
    assert_send::<actor::Context<(), context::ThreadSafe>>();
    assert_sync::<actor::Context<(), context::ThreadSafe>>();
}

async fn local_actor(mut ctx: actor::Context<usize>) -> Result<(), !> {
    assert_eq!(ctx.try_receive_next(), Err(RecvError::Empty));

    let msg = ctx.receive_next().await.unwrap();
    assert_eq!(msg, 123);

    assert_eq!(ctx.receive_next().await, Err(NoMessages));
    assert_eq!(ctx.try_receive_next(), Err(RecvError::Disconnected));
    Ok(())
}

#[test]
fn test_local_actor_context() {
    let local_actor = local_actor as fn(_) -> _;
    let (actor, actor_ref) = init_local_actor(local_actor, ()).unwrap();
    let mut actor = Box::pin(actor);

    // Inbox should be empty initially.
    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Pending);

    // Any send message should be receivable.
    actor_ref.try_send(123usize).unwrap();
    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Pending);

    // Once all actor references are dropped
    drop(actor_ref);
    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
}
