//! Tests for the `actor::Context`.

#![cfg(feature = "test")]

use std::pin::Pin;
use std::task::Poll;

use heph::actor::{self, context, NoMessages, RecvError};
use heph::rt::Signal;
use heph::test::{init_local_actor, poll_actor};

use crate::util::{assert_send, assert_sync};

#[test]
fn thread_safe_is_send_sync() {
    assert_send::<actor::Context<(), context::ThreadSafe>>();
    assert_sync::<actor::Context<(), context::ThreadSafe>>();
}

async fn local_actor_context_actor(mut ctx: actor::Context<usize>) -> Result<(), !> {
    assert_eq!(ctx.try_receive_next(), Err(RecvError::Empty));

    let msg = ctx.receive_next().await.unwrap();
    assert_eq!(msg, 123);

    assert_eq!(ctx.receive_next().await, Err(NoMessages));
    assert_eq!(ctx.try_receive_next(), Err(RecvError::Disconnected));
    Ok(())
}

#[test]
fn local_actor_context() {
    let local_actor_context_actor = local_actor_context_actor as fn(_) -> _;
    let (actor, actor_ref) = init_local_actor(local_actor_context_actor, ()).unwrap();
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

async fn actor_ref_actor(mut ctx: actor::Context<usize>) -> Result<(), !> {
    assert_eq!(ctx.receive_next().await, Err(NoMessages));

    // Send a message to ourselves.
    let self_ref = ctx.actor_ref();
    self_ref.send(123usize).await.unwrap();
    let msg = ctx.receive_next().await.unwrap();
    assert_eq!(msg, 123);

    Ok(())
}

#[test]
fn actor_ref() {
    let actor_ref_actor = actor_ref_actor as fn(_) -> _;
    let (actor, actor_ref) = init_local_actor(actor_ref_actor, ()).unwrap();
    let mut actor = Box::pin(actor);

    drop(actor_ref);
    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
}
