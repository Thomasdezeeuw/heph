//! Tests related to `ActorRef` and `ActorGroup`.

#![cfg(feature = "test")]

use std::fmt;
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::task::Poll;

use heph::test::{init_local_actor, poll_actor};
use heph::{actor, ActorRef};

const MSGS: &[&str] = &["Hello world", "Hello mars", "Hello moon"];

async fn expect_msgs<M>(mut ctx: actor::Context<M>, expected: Vec<M>) -> Result<(), !>
where
    M: Eq + fmt::Debug,
{
    for expected in expected {
        let got = ctx.receive_next().await;
        assert_eq!(got, expected);
    }
    Ok(())
}

#[test]
fn try_send() {
    let expect_msgs = expect_msgs as fn(_, _) -> _;
    let (actor, actor_ref) = init_local_actor(expect_msgs, MSGS.to_vec()).unwrap();
    let mut actor = Box::pin(actor);

    for msg in MSGS {
        actor_ref.try_send(*msg).unwrap();
    }

    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
}

async fn relay_msgs<M>(
    _: actor::Context<M>,
    mut relay_ref: ActorRef<M>,
    msgs: Vec<M>,
) -> Result<(), !>
where
    M: Eq + fmt::Debug + Unpin,
{
    for msg in msgs {
        relay_ref.send(msg).await.unwrap()
    }
    Ok(())
}

#[test]
fn send() {
    let expect_msgs = expect_msgs as fn(_, _) -> _;
    let (actor, actor_ref) = init_local_actor(expect_msgs, MSGS.to_vec()).unwrap();
    let mut actor = Box::pin(actor);

    let relay_msgs = relay_msgs as fn(_, _, _) -> _;
    let (relay_actor, _) = init_local_actor(relay_msgs, (actor_ref, MSGS.to_vec())).unwrap();
    let mut relay_actor = Box::pin(relay_actor);

    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Pending);
    assert_eq!(
        poll_actor(Pin::as_mut(&mut relay_actor)),
        Poll::Ready(Ok(()))
    );
    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
}

#[test]
fn map() {
    let expect_msgs = expect_msgs as fn(_, _) -> _;
    let expected = MSGS.iter().map(|s| (*s).to_owned()).collect();
    let (actor, actor_ref): (_, ActorRef<String>) =
        init_local_actor(expect_msgs, expected).unwrap();
    let mut actor = Box::pin(actor);

    let actor_ref: ActorRef<&str> = actor_ref.map();
    for msg in MSGS {
        actor_ref.try_send(*msg).unwrap();
    }

    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
}

#[test]
fn try_map() {
    let expect_msgs = expect_msgs as fn(_, _) -> _;
    let expected = vec![
        NonZeroUsize::new(1).unwrap(),
        NonZeroUsize::new(2).unwrap(),
        NonZeroUsize::new(3).unwrap(),
    ];
    let (actor, actor_ref): (_, ActorRef<NonZeroUsize>) =
        init_local_actor(expect_msgs, expected).unwrap();
    let mut actor = Box::pin(actor);

    let actor_ref: ActorRef<usize> = actor_ref.try_map();
    assert!(actor_ref.try_send(0usize).is_err());
    for msg in 1..4usize {
        actor_ref.try_send(msg).unwrap();
    }

    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
}
