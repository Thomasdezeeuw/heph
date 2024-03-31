//! Tests related to `ActorGroup`.

use heph::actor_ref::{ActorGroup, Delivery, SendError};
use heph::future::{ActorFuture, ActorFutureBuilder, InboxSize};
use heph::supervisor::NoSupervisor;
use heph::{actor, actor_fn};

use crate::util::{assert_send, assert_size, assert_sync, block_on};

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

#[test]
fn make_unique_empty() {
    let mut group = ActorGroup::<()>::empty();
    assert_eq!(group.len(), 0);
    group.make_unique();
    assert_eq!(group.len(), 0);
}

#[test]
fn try_send_to_one() {
    let (future, actor_ref) = ActorFuture::new(NoSupervisor, actor_fn(count_actor), 1).unwrap();

    let group = ActorGroup::from(actor_ref);
    assert_eq!(group.try_send_to_one(()), Ok(()));
    drop(group);

    block_on(future);
}

#[test]
fn try_send_to_one_full_inbox() {
    let (future, actor_ref) = ActorFutureBuilder::new()
        .with_inbox_size(InboxSize::ONE)
        .build(NoSupervisor, actor_fn(count_actor), 1)
        .unwrap();

    let group = ActorGroup::from(actor_ref);
    assert_eq!(group.try_send_to_one(()), Ok(()));
    assert_eq!(group.try_send_to_one(()), Err(SendError));
    drop(group);

    block_on(future);
}

#[test]
fn try_send_to_one_empty() {
    let group = ActorGroup::<()>::empty();
    assert_eq!(group.try_send_to_one(()), Err(SendError));
}

#[test]
fn try_send_to_all() {
    let (future1, actor_ref1) = ActorFuture::new(NoSupervisor, actor_fn(count_actor), 1).unwrap();
    let (future2, actor_ref2) = ActorFuture::new(NoSupervisor, actor_fn(count_actor), 1).unwrap();

    let group = ActorGroup::new([actor_ref1, actor_ref2]);
    assert_eq!(group.try_send_to_all(()), Ok(()));
    drop(group);

    block_on(future1);
    block_on(future2);
}

#[test]
fn try_send_to_all_full_inbox() {
    let (future1, actor_ref1) = ActorFutureBuilder::new()
        .with_inbox_size(InboxSize::ONE)
        .build(NoSupervisor, actor_fn(count_actor), 1)
        .unwrap();
    let (future2, actor_ref2) = ActorFutureBuilder::new()
        .with_inbox_size(InboxSize::ONE)
        .build(NoSupervisor, actor_fn(count_actor), 1)
        .unwrap();

    let group = ActorGroup::new([actor_ref1, actor_ref2]);
    assert_eq!(group.try_send_to_all(()), Ok(()));
    assert_eq!(group.try_send_to_all(()), Ok(()));
    drop(group);

    block_on(future1);
    block_on(future2);
}

#[test]
fn try_send_to_all_empty() {
    let group = ActorGroup::<()>::empty();
    assert_eq!(group.try_send_to_all(()), Err(SendError));
}

async fn count_actor(mut ctx: actor::Context<(), ()>, expected_amount: usize) {
    let mut amount = 0;
    while let Ok(()) = ctx.receive_next().await {
        amount += 1;
    }
    assert_eq!(amount, expected_amount);
}
