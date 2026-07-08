//! Tests related to `ActorGroup`.

use std::convert::Infallible;
use std::fmt;
use std::pin::{Pin, pin};
use std::task::Poll;

use heph::actor_ref::{ActorGroup, SendError};
use heph::future::{ActorFuture, ActorFutureBuilder, InboxSize};
use heph::supervisor::NoSupervisor;
use heph::{actor, actor_fn};

use crate::util::{
    assert_send, assert_size, assert_sync, block_on, block_on_many, poll, poll_once,
};

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
    let (future, actor_ref) = ActorFuture::new(NoSupervisor, actor_fn(count_actor), 1);

    let group = ActorGroup::from(actor_ref);
    assert_eq!(group.try_send_to_one(()), Ok(()));
    drop(group);

    block_on(future);
}

#[test]
fn try_send_to_one_full_inbox() {
    let (future, actor_ref) = ActorFutureBuilder::new()
        .with_inbox_size(InboxSize::ONE)
        .build(NoSupervisor, actor_fn(count_actor), 1);

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
fn send_to_one() {
    let (future, actor_ref) = ActorFuture::new(NoSupervisor, actor_fn(count_actor), 1);

    let group = ActorGroup::from(actor_ref);
    assert_eq!(block_on(group.send_to_one(())), Ok(()));
    drop(group);

    block_on(future);
}

#[test]
fn send_to_one_full_inbox() {
    let (future, actor_ref) = ActorFutureBuilder::new()
        .with_inbox_size(InboxSize::ONE)
        .build(NoSupervisor, actor_fn(count_actor), 1);
    let mut future = pin!(future);

    let group = ActorGroup::from(actor_ref);
    assert_eq!(block_on(group.send_to_one(())), Ok(()));

    {
        let mut send_future = pin!(group.send_to_one(()));
        poll_once(send_future.as_mut()); // Should return Poll::Pending.

        // Emptying the actor's inbox should allow us to send the message.
        poll_once(future.as_mut());
        assert_eq!(block_on(send_future), Ok(()));
    } // Drops `send_future`, to allow us to drop `group`.
    drop(group);

    block_on(future);
}

#[test]
fn send_to_one_empty() {
    let group = ActorGroup::<()>::empty();
    assert_eq!(block_on(group.send_to_one(())), Err(SendError));
}

#[test]
fn try_send_to_all() {
    let (future1, actor_ref1) = ActorFuture::new(NoSupervisor, actor_fn(count_actor), 1);
    let (future2, actor_ref2) = ActorFuture::new(NoSupervisor, actor_fn(count_actor), 1);

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
        .build(NoSupervisor, actor_fn(count_actor), 1);
    let (future2, actor_ref2) = ActorFutureBuilder::new()
        .with_inbox_size(InboxSize::ONE)
        .build(NoSupervisor, actor_fn(count_actor), 1);

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

async fn expect_msgs<M>(mut ctx: actor::Context<M>, expected: Vec<M>)
where
    M: Eq + fmt::Debug,
{
    for expected in expected {
        let got = ctx.receive_next().await.expect("missing message");
        assert_eq!(got, expected);
    }
}

//
//
//
//
//
//
//
//
//
//
//

#[test]
fn from_actor_ref() {
    let expect_msgs = actor_fn(expect_msgs);
    let (actor, actor_ref) = ActorFuture::new(NoSupervisor, expect_msgs, vec![()]);
    let actor = pin!(actor);

    let group = ActorGroup::from(actor_ref);
    assert_eq!(group.len(), 1);

    assert!(group.try_send_to_all(()).is_ok());
    assert_eq!(poll(actor), Poll::Ready(()));
}

#[test]
fn from_iter() {
    let mut actors: Vec<Pin<Box<dyn Future<Output = ()>>>> = Vec::new();
    let group: ActorGroup<()> = (0..3)
        .into_iter()
        .map(|_| {
            let expect_msgs = actor_fn(expect_msgs);
            let (actor, actor_ref) = ActorFuture::new(NoSupervisor, expect_msgs, vec![()]);
            actors.push(Box::pin(actor));
            actor_ref
        })
        .collect();
    assert_eq!(group.len(), 3);

    assert!(group.try_send_to_all(()).is_ok());
    block_on_many(actors);
}

#[test]
fn add_to_empty_group() {
    let mut group = ActorGroup::<usize>::empty();

    let expect_msgs = actor_fn(expect_msgs);
    let (actor, actor_ref) = ActorFuture::new(NoSupervisor, expect_msgs, vec![1usize]);
    let mut actor = Box::pin(actor);
    group.add(actor_ref);

    group.try_send_to_all(1_usize).unwrap();
    assert_eq!(poll(actor.as_mut()), Poll::Ready(()));
}

#[test]
fn add_actor_to_group() {
    let expect_msgs = actor_fn(expect_msgs);
    let mut actors = Vec::new();
    let mut group: ActorGroup<usize> = (0..3)
        .into_iter()
        .map(|_| {
            let (actor, actor_ref) =
                ActorFuture::new(NoSupervisor, expect_msgs, vec![123_usize, 456, 789]);
            actors.push(Box::pin(actor));
            actor_ref
        })
        .collect();

    // Add new actor.
    let (actor, actor_ref) = ActorFuture::new(NoSupervisor, expect_msgs, vec![123_usize, 456, 789]);
    actors.push(Box::pin(actor));
    group.add(actor_ref);

    group.try_send_to_all(123_usize).unwrap();
    group.try_send_to_all(456_usize).unwrap();
    group.try_send_to_all(789_usize).unwrap();
    for mut actor in actors {
        assert_eq!(poll(actor.as_mut()), Poll::Ready(()));
    }
}

#[test]
fn add_unique() {
    let expect_msgs = actor_fn(expect_msgs);
    let mut actors = Vec::new();
    let mut group = ActorGroup::empty();
    let mut add_later = Vec::new();
    const N: usize = 3;
    for _ in 0..N {
        let (actor, actor_ref) = ActorFuture::new(NoSupervisor, expect_msgs, vec![123usize]);
        actors.push(Box::pin(actor));
        group.add_unique(actor_ref.clone());
        group.add_unique(actor_ref.clone());
        add_later.push(actor_ref);
    }
    for actor_ref in add_later.into_iter().rev() {
        group.add_unique(actor_ref);
    }

    assert_eq!(group.len(), N);

    group.try_send_to_all(123usize).unwrap();
    for mut actor in actors {
        assert_eq!(poll(actor.as_mut()), Poll::Ready(()));
    }
}

#[test]
fn extend_empty_actor_group() {
    let mut group = ActorGroup::<usize>::empty();

    let mut actors = Vec::new();
    group.extend((0..3).into_iter().map(|_| {
        let expect_msgs = actor_fn(expect_msgs);
        let (actor, actor_ref) = ActorFuture::new(NoSupervisor, expect_msgs, vec![123usize]);
        actors.push(Box::pin(actor));
        actor_ref
    }));

    group.try_send_to_all(123usize).unwrap();
    for mut actor in actors {
        assert_eq!(poll(actor.as_mut()), Poll::Ready(()));
    }
}

#[test]
fn extend_actor_group() {
    let expect_msgs = actor_fn(expect_msgs);
    let mut actors = Vec::new();
    let mut group: ActorGroup<usize> = (0..3)
        .into_iter()
        .map(|_| {
            let (actor, actor_ref) = ActorFuture::new(NoSupervisor, expect_msgs, vec![123usize]);
            actors.push(Box::pin(actor));
            actor_ref
        })
        .collect();

    group.extend((0..3).into_iter().map(|_| {
        let (actor, actor_ref) = ActorFuture::new(NoSupervisor, expect_msgs, vec![123usize]);
        actors.push(Box::pin(actor));
        actor_ref
    }));

    group.try_send_to_all(123usize).unwrap();
    for mut actor in actors {
        assert_eq!(poll(actor.as_mut()), Poll::Ready(()));
    }
}

#[test]
fn remove() {
    let expect_msgs = actor_fn(expect_msgs);
    let mut actor_refs = Vec::new();
    let mut group = ActorGroup::empty();
    for _ in 0..3 {
        let (_, actor_ref) = ActorFuture::new(NoSupervisor, expect_msgs, vec![123usize]);
        actor_refs.push(actor_ref.clone());
        group.add(actor_ref);
    }

    assert_eq!(group.len(), actor_refs.len());

    // Remove an unknown actor ref should do nothing.
    let (_, actor_ref) = ActorFuture::new(NoSupervisor, expect_msgs, vec![123usize]);
    group.remove(&actor_ref);
    assert_eq!(group.len(), actor_refs.len());

    let mut iter = actor_refs.into_iter();
    while let Some(actor_ref) = iter.next() {
        group.remove(&actor_ref);
        assert_eq!(group.len(), iter.len());
    }
}

#[test]
fn remove_disconnected() {
    let mut actors = Vec::new();
    let mut group = ActorGroup::empty();
    for _ in 0..3 {
        let expect_msgs = actor_fn(expect_msgs);
        let (actor, actor_ref) = ActorFuture::new(NoSupervisor, expect_msgs, vec![123usize]);
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
fn make_unique() {
    let expect_msgs = actor_fn(expect_msgs);
    let mut actors = Vec::new();
    let mut group = ActorGroup::empty();
    let mut add_later = Vec::new();
    const N: usize = 3;
    for _ in 0..N {
        let (actor, actor_ref) = ActorFuture::new(NoSupervisor, expect_msgs, vec![123usize]);
        actors.push(Box::pin(actor));
        group.add(actor_ref.clone());
        group.add(actor_ref.clone());
        add_later.push(actor_ref);
    }
    for actor_ref in add_later.into_iter().rev() {
        group.add(actor_ref);
    }

    assert_eq!(group.len(), 3 * N);
    group.make_unique();
    assert_eq!(group.len(), N);

    group.try_send_to_all(123usize).unwrap();
    for mut actor in actors {
        assert_eq!(poll(actor.as_mut()), Poll::Ready(()));
    }
}

#[test]
fn make_unique_one() {
    let expect_msgs = actor_fn(expect_msgs);
    let mut group = ActorGroup::empty();
    let (actor, actor_ref) = ActorFuture::new(NoSupervisor, expect_msgs, vec![123usize]);
    let mut actor = Box::pin(actor);
    group.add(actor_ref.clone());
    group.add(actor_ref);

    assert_eq!(group.len(), 2);
    group.make_unique();
    assert_eq!(group.len(), 1);

    group.try_send_to_all(123usize).unwrap();
    assert_eq!(poll(actor.as_mut()), Poll::Ready(()));
}

#[test]
fn send_delivery_to_all() {
    let mut actors = Vec::new();
    let mut group = ActorGroup::empty();
    for _ in 0..10 {
        let expect_msgs = actor_fn(expect_msgs);
        let (actor, actor_ref) = ActorFuture::new(NoSupervisor, expect_msgs, vec![123usize]);
        actors.push(Box::pin(actor));
        group.add(actor_ref);
    }

    assert!(group.try_send_to_all(123usize).is_ok());
    for mut actor in actors {
        assert_eq!(poll(actor.as_mut()), Poll::Ready(()));
    }
}

#[test]
fn send_delivery_to_one() {
    const N: usize = 10;
    let mut actors = Vec::new();
    let mut group = ActorGroup::empty();
    for _ in 0..N {
        let expect_msgs = actor_fn(expect_msgs);
        let (actor, actor_ref) = ActorFuture::new(NoSupervisor, expect_msgs, vec![123usize]);
        actors.push(Box::pin(actor));
        group.add(actor_ref);
    }

    // NOTE: sending order is not guaranteed so this test is too strict.
    for mut actor in actors {
        assert!(group.try_send_to_one(123usize).is_ok());

        assert_eq!(poll(actor.as_mut()), Poll::Ready(()));
    }
}

async fn stop_on_run(ctx: actor::Context<Infallible>) {
    drop(ctx);
}

fn join_all(n: usize) {
    let mut actors = Vec::new();
    let group: ActorGroup<_> = (0..n)
        .into_iter()
        .map(|_| {
            let stop_on_run = actor_fn(stop_on_run);
            let (actor, actor_ref) = ActorFuture::new(NoSupervisor, stop_on_run, ());
            actors.push(Box::pin(actor));
            actor_ref
        })
        .collect();
    assert_eq!(group.len(), n);

    let future = group.join_all();
    let mut future = Box::pin(future);

    assert_eq!(poll(future.as_mut()), Poll::Pending);

    for mut actor in actors {
        assert_eq!(poll(actor.as_mut()), Poll::Ready(()));
    }
    assert_eq!(poll(future.as_mut()), Poll::Ready(()));
}

#[test]
fn join_all_one() {
    join_all(1);
}

#[test]
fn join_all_two() {
    join_all(2);
}

#[test]
fn join_all_five() {
    join_all(5);
}

#[test]
fn join_all_ten() {
    join_all(10);
}

#[test]
fn join_all_after_actor_dropped() {
    let stop_on_run = actor_fn(stop_on_run);
    let (actor, actor_ref) = ActorFuture::new(NoSupervisor, stop_on_run, ());
    let mut actor = Box::pin(actor);

    assert_eq!(poll(actor.as_mut()), Poll::Ready(()));
    drop(actor);

    let future = actor_ref.join();
    let mut future = Box::pin(future);
    assert_eq!(poll(future.as_mut()), Poll::Ready(()));
}

#[test]
fn join_all_before_one_actor_finished() {
    fn test(even: bool) {
        let mut actors = Vec::new();
        let group: ActorGroup<_> = (0..10)
            .into_iter()
            .map(|_| {
                let stop_on_run = actor_fn(stop_on_run);
                let (actor, actor_ref) = ActorFuture::new(NoSupervisor, stop_on_run, ());
                actors.push(Box::pin(actor));
                actor_ref
            })
            .collect();

        let future = group.join_all();
        let mut future = Box::pin(future);

        // Run half of the actors.
        let actors: Vec<_> = actors
            .into_iter()
            .enumerate()
            .filter_map(|(i, mut actor)| {
                if (i % 2 == 0) == even {
                    assert_eq!(poll(actor.as_mut()), Poll::Ready(()));
                    None
                } else {
                    Some(actor)
                }
            })
            .collect();

        assert_eq!(poll(future.as_mut()), Poll::Pending);

        for mut actor in actors {
            assert_eq!(poll(actor.as_mut()), Poll::Ready(()));
        }
        assert_eq!(poll(future.as_mut()), Poll::Ready(()));
    }

    test(true);
    test(false);
}
