//! Tests for the `actor::Context`.

#![cfg(feature = "test")]

use std::pin::Pin;
use std::task::Poll;

use heph::actor::{self, NoMessages, RecvError};
use heph::rt::{ActorOptions, Runtime, Signal, ThreadSafe};
use heph::supervisor::NoSupervisor;
use heph::test::{init_local_actor, poll_actor};

use crate::util::{assert_send, assert_sync};

#[test]
fn thread_safe_is_send_sync() {
    assert_send::<actor::Context<(), ThreadSafe>>();
    assert_sync::<actor::Context<(), ThreadSafe>>();
}

async fn local_actor_context_actor(mut ctx: actor::Context<usize>) {
    assert_eq!(ctx.try_receive_next(), Err(RecvError::Empty));

    let msg = ctx.receive_next().await.unwrap();
    assert_eq!(msg, 123);

    assert_eq!(ctx.receive_next().await, Err(NoMessages));
    assert_eq!(ctx.try_receive_next(), Err(RecvError::Disconnected));
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

async fn actor_ref_actor(mut ctx: actor::Context<usize>) {
    assert_eq!(ctx.receive_next().await, Err(NoMessages));

    // Send a message to ourselves.
    let self_ref = ctx.actor_ref();
    self_ref.send(123usize).await.unwrap();
    let msg = ctx.receive_next().await.unwrap();
    assert_eq!(msg, 123);
}

#[test]
fn actor_ref() {
    let actor_ref_actor = actor_ref_actor as fn(_) -> _;
    let (actor, actor_ref) = init_local_actor(actor_ref_actor, ()).unwrap();
    let mut actor = Box::pin(actor);

    drop(actor_ref);
    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
}

async fn runtime_actor(mut ctx: actor::Context<Signal>) {
    let actor_ref = ctx.actor_ref();
    ctx.runtime().receive_signals(actor_ref);
}

#[test]
fn runtime() {
    let runtime_actor = runtime_actor as fn(_) -> _;
    let (actor, actor_ref) = init_local_actor(runtime_actor, ()).unwrap();
    let mut actor = Box::pin(actor);

    drop(actor_ref);
    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
}

async fn thread_safe_try_spawn_actor(mut ctx: actor::Context<usize, ThreadSafe>) {
    let actor_ref1 = ctx
        .try_spawn(
            NoSupervisor,
            spawned_actor1 as fn(_) -> _,
            (),
            ActorOptions::default(),
        )
        .unwrap();
    let actor_ref2 = ctx.spawn(
        NoSupervisor,
        spawned_actor1 as fn(_) -> _,
        (),
        ActorOptions::default(),
    );

    actor_ref1.send(123usize).await.unwrap();
    actor_ref2.send(123usize).await.unwrap();
}

async fn spawned_actor1(mut ctx: actor::Context<usize, ThreadSafe>) {
    let msg = ctx.receive_next().await.unwrap();
    assert_eq!(msg, 123);
}

#[test]
fn thread_safe_try_spawn() {
    let thread_safe_try_spawn_actor = thread_safe_try_spawn_actor as fn(_) -> _;
    let mut runtime = Runtime::new().unwrap();
    let _ = runtime.spawn(
        NoSupervisor,
        thread_safe_try_spawn_actor,
        (),
        ActorOptions::default(),
    );
    runtime.start().unwrap();
}
