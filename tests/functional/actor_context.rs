//! Tests for the `actor::Context`.

use std::pin::{Pin, pin};
use std::task::Poll;

use heph::ActorFuture;
use heph::actor::{self, NoMessages, RecvError, actor_fn};
use heph::supervisor::NoSupervisor;

use crate::util::{assert_send, assert_sync, poll};

#[test]
fn context_is_send_sync() {
    assert_send::<actor::Context<()>>();
    assert_sync::<actor::Context<()>>();
}

async fn actor(mut ctx: actor::Context<usize>) {
    assert_eq!(ctx.try_receive_next(), Err(RecvError::Empty));

    let msg = ctx.receive_next().await.unwrap();
    assert_eq!(msg, 123);

    assert_eq!(ctx.receive_next().await, Err(NoMessages));
    assert_eq!(ctx.try_receive_next(), Err(RecvError::Disconnected));
}

#[test]
fn inbox() {
    let actor = actor_fn(actor);
    let (actor, actor_ref) = ActorFuture::new(NoSupervisor, actor, ());
    let mut actor = pin!(actor);

    // Inbox should be empty initially.
    assert_eq!(poll(actor.as_mut()), Poll::Pending);

    // Any send message should be receivable.
    actor_ref.try_send(123usize).unwrap();
    assert_eq!(poll(actor.as_mut()), Poll::Pending);

    // Once all actor references are dropped
    drop(actor_ref);
    assert_eq!(poll(actor.as_mut()), Poll::Ready(()));
}

async fn actor_self_send(mut ctx: actor::Context<usize>) {
    assert_eq!(ctx.receive_next().await, Err(NoMessages));

    // Send a message to ourselves.
    let self_ref = ctx.actor_ref();
    self_ref.send(123usize).await.unwrap();
    let msg = ctx.receive_next().await.unwrap();
    assert_eq!(msg, 123);
}

#[test]
fn send_message_to_self() {
    let actor_self_send = actor_fn(actor_self_send);
    let (actor, actor_ref) = ActorFuture::new(NoSupervisor, actor_self_send, ());
    let mut actor = pin!(actor);

    drop(actor_ref);
    assert_eq!(poll(Pin::as_mut(&mut actor)), Poll::Ready(()));
}
