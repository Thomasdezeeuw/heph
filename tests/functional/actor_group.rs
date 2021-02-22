//! Tests related to `ActorGroup`.

#![cfg(feature = "test")]

use std::fmt;
use std::pin::Pin;
use std::task::Poll;

use heph::actor;
use heph::actor_ref::{ActorGroup, Delivery};
use heph::test::{init_local_actor, poll_actor};

use crate::util::{assert_send, assert_size, assert_sync};

#[test]
fn size() {
    assert_size::<ActorGroup<()>>(32);
}

#[test]
fn is_send_sync() {
    assert_send::<ActorGroup<()>>();
    assert_sync::<ActorGroup<()>>();

    // UnsafeCell is !Sync and Send, our reference should still be Send and
    // Sync.
    assert_send::<ActorGroup<std::cell::UnsafeCell<()>>>();
    assert_sync::<ActorGroup<std::cell::UnsafeCell<()>>>();
}

#[test]
fn empty() {
    let group = ActorGroup::<()>::empty();
    assert!(group.try_send((), Delivery::ToAll).is_err());
    assert!(group.try_send((), Delivery::ToOne).is_err());
    assert_eq!(group.len(), 0);
    assert!(group.is_empty());
}

async fn expect_msgs<M>(mut ctx: actor::Context<M>, expected: Vec<M>)
where
    M: Eq + fmt::Debug,
{
    for expected in expected {
        let got = ctx.receive_next().await.expect("missing message");
        assert_eq!(got, expected);
    }
}

#[test]
fn new() {
    let mut actors = Vec::new();
    let mut actor_refs = Vec::new();
    for _ in 0..3 {
        let expect_msgs = expect_msgs as fn(_, _) -> _;
        let (actor, actor_ref) = init_local_actor(expect_msgs, vec![123usize]).unwrap();
        actors.push(Box::pin(actor));
        actor_refs.push(actor_ref);
    }

    let group = ActorGroup::new(actor_refs);
    assert_eq!(group.len(), 3);
    assert!(!group.is_empty());

    assert!(group.try_send(123usize, Delivery::ToAll).is_ok());
    for mut actor in actors {
        assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
    }
}

#[test]
fn from_iter() {
    let mut actors = Vec::new();
    let group: ActorGroup<()> = (0..3)
        .into_iter()
        .map(|_| {
            let expect_msgs = expect_msgs as fn(_, _) -> _;
            let (actor, actor_ref) = init_local_actor(expect_msgs, vec![()]).unwrap();
            actors.push(Box::pin(actor));
            actor_ref
        })
        .collect();
    assert_eq!(group.len(), 3);

    assert!(group.try_send((), Delivery::ToAll).is_ok());
    for mut actor in actors {
        assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
    }
}

#[test]
fn add_to_empty_group() {
    let mut group = ActorGroup::<usize>::empty();

    let expect_msgs = expect_msgs as fn(_, _) -> _;
    let (actor, actor_ref) = init_local_actor(expect_msgs, vec![1usize]).unwrap();
    let mut actor = Box::pin(actor);
    group.add(actor_ref);

    group.try_send(1usize, Delivery::ToAll).unwrap();
    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
}

#[test]
fn add_actor_to_group() {
    let expect_msgs = expect_msgs as fn(_, _) -> _;
    let mut actors = Vec::new();
    let mut group: ActorGroup<usize> = (0..3)
        .into_iter()
        .map(|_| {
            let (actor, actor_ref) =
                init_local_actor(expect_msgs, vec![123usize, 456, 789]).unwrap();
            actors.push(Box::pin(actor));
            actor_ref
        })
        .collect();

    // Add new actor.
    let (actor, actor_ref) = init_local_actor(expect_msgs, vec![123usize, 456, 789]).unwrap();
    actors.push(Box::pin(actor));
    group.add(actor_ref);

    group.try_send(123usize, Delivery::ToAll).unwrap();
    group.try_send(456usize, Delivery::ToAll).unwrap();
    group.try_send(789usize, Delivery::ToAll).unwrap();
    for mut actor in actors {
        assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
    }
}

#[test]
fn extend_empty_actor_group() {
    let mut group = ActorGroup::<usize>::empty();

    let mut actors = Vec::new();
    group.extend((0..3).into_iter().map(|_| {
        let expect_msgs = expect_msgs as fn(_, _) -> _;
        let (actor, actor_ref) = init_local_actor(expect_msgs, vec![123usize]).unwrap();
        actors.push(Box::pin(actor));
        actor_ref
    }));

    group.try_send(123usize, Delivery::ToAll).unwrap();
    for mut actor in actors {
        assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
    }
}

#[test]
fn extend_actor_group() {
    let expect_msgs = expect_msgs as fn(_, _) -> _;
    let mut actors = Vec::new();
    let mut group: ActorGroup<usize> = (0..3)
        .into_iter()
        .map(|_| {
            let (actor, actor_ref) = init_local_actor(expect_msgs, vec![123usize]).unwrap();
            actors.push(Box::pin(actor));
            actor_ref
        })
        .collect();

    group.extend((0..3).into_iter().map(|_| {
        let (actor, actor_ref) = init_local_actor(expect_msgs, vec![123usize]).unwrap();
        actors.push(Box::pin(actor));
        actor_ref
    }));

    group.try_send(123usize, Delivery::ToAll).unwrap();
    for mut actor in actors {
        assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
    }
}

#[test]
fn remove() {
    let expect_msgs = expect_msgs as fn(_, _) -> _;
    let mut actor_refs = Vec::new();
    let mut group = ActorGroup::empty();
    for _ in 0..3 {
        let (_, actor_ref) = init_local_actor(expect_msgs, vec![123usize]).unwrap();
        actor_refs.push(actor_ref.clone());
        group.add(actor_ref);
    }

    assert_eq!(group.len(), actor_refs.len());

    // Remove an unknown actor ref should do nothing.
    let (_, actor_ref) = init_local_actor(expect_msgs, vec![123usize]).unwrap();
    group.remove(actor_ref);
    assert_eq!(group.len(), actor_refs.len());

    let mut iter = actor_refs.into_iter();
    while let Some(actor_ref) = iter.next() {
        group.remove(actor_ref);
        assert_eq!(group.len(), iter.len());
    }
}

#[test]
fn remove_disconnected() {
    let mut actors = Vec::new();
    let mut group = ActorGroup::empty();
    for _ in 0..3 {
        let expect_msgs = expect_msgs as fn(_, _) -> _;
        let (actor, actor_ref) = init_local_actor(expect_msgs, vec![123usize]).unwrap();
        actors.push(Box::pin(actor));
        group.add(actor_ref);
    }

    assert_eq!(group.len(), actors.len());

    // All actors are still connected.
    group.remove_disconnected();
    assert_eq!(group.len(), actors.len());

    let mut iter = actors.into_iter();
    while let Some(actor) = iter.next() {
        drop(actor);
        group.remove_disconnected();
        assert_eq!(group.len(), iter.len());
    }
}

#[test]
fn send_delivery_to_all() {
    let mut actors = Vec::new();
    let mut group = ActorGroup::empty();
    for _ in 0..10 {
        let expect_msgs = expect_msgs as fn(_, _) -> _;
        let (actor, actor_ref) = init_local_actor(expect_msgs, vec![123usize]).unwrap();
        actors.push(Box::pin(actor));
        group.add(actor_ref);
    }

    assert!(group.try_send(123usize, Delivery::ToAll).is_ok());
    for mut actor in actors {
        assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
    }
}

#[test]
fn send_delivery_to_one() {
    const N: usize = 10;
    let mut actors = Vec::new();
    let mut group = ActorGroup::empty();
    for _ in 0..N {
        let expect_msgs = expect_msgs as fn(_, _) -> _;
        let (actor, actor_ref) = init_local_actor(expect_msgs, vec![123usize]).unwrap();
        actors.push(Box::pin(actor));
        group.add(actor_ref);
    }

    // NOTE: sending order is not gauranteed so this test is too strict.
    for mut actor in actors {
        assert!(group.try_send(123usize, Delivery::ToOne).is_ok());

        assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
    }
}
