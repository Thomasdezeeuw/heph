//! Tests related to `ActorRef`.

use std::convert::Infallible;
use std::fmt;
use std::future::ready;
use std::num::NonZeroUsize;
use std::pin::{pin, Pin};
use std::task::Poll;

use heph::actor::{self, actor_fn};
use heph::actor_ref::{ActorRef, Join, RpcError, RpcMessage, SendError, SendValue};
use heph::messages::from_message;
use heph::supervisor::NoSupervisor;
use heph_rt::spawn::options::Priority;
use heph_rt::spawn::ActorOptions;
use heph_rt::test::{init_local_actor, poll_actor, poll_future};
use heph_rt::{Runtime, ThreadLocal};

use crate::util::{assert_send, assert_size, assert_sync, pending_once};

/// Default size of the inbox, keep in sync with the inbox crate.
const INBOX_SIZE: usize = 8;

const MSGS: &[&str] = &["Hello world", "Hello mars", "Hello moon"];

#[test]
fn size() {
    assert_size::<ActorRef<()>>(24);
    assert_size::<SendValue<'_, ()>>(40);
    assert_size::<Join<'_, ()>>(32);
}

#[test]
fn is_send_sync() {
    assert_send::<ActorRef<()>>();
    assert_sync::<ActorRef<()>>();

    // UnsafeCell is !Sync and Send, our reference should still be Send and
    // Sync.
    assert_send::<ActorRef<std::cell::UnsafeCell<()>>>();
    assert_sync::<ActorRef<std::cell::UnsafeCell<()>>>();
}

#[test]
fn send_value_future_is_send_sync() {
    assert_send::<SendValue<()>>();
    assert_sync::<SendValue<()>>();
}

async fn expect_msgs<M>(mut ctx: actor::Context<M, ThreadLocal>, expected: Vec<M>)
where
    M: Eq + fmt::Debug,
{
    for expected in expected {
        let got = ctx.receive_next().await.expect("missing message");
        assert_eq!(got, expected);
    }
}

#[test]
fn try_send() {
    let expect_msgs = actor_fn(expect_msgs);
    let (actor, actor_ref) = init_local_actor(expect_msgs, MSGS.to_vec()).unwrap();
    let mut actor = Box::pin(actor);

    for msg in MSGS {
        actor_ref.try_send(*msg).unwrap();
    }

    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
}

#[test]
fn try_send_full_inbox() {
    let expected: Vec<usize> = (0..INBOX_SIZE + 2).collect();
    let expect_msgs = actor_fn(expect_msgs);
    let (actor, actor_ref) = init_local_actor(expect_msgs, expected.clone()).unwrap();
    let mut actor = Box::pin(actor);

    let mut iter = expected.into_iter();
    for msg in iter.by_ref().take(INBOX_SIZE) {
        actor_ref.try_send(msg).unwrap();
    }

    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Pending);

    for msg in iter {
        actor_ref.try_send(msg).unwrap();
    }
    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
}

#[test]
fn try_send_disconnected() {
    let expect_msgs = actor_fn::<_, usize, _, _, _>(expect_msgs);
    let (actor, actor_ref) = init_local_actor(expect_msgs, Vec::new()).unwrap();
    drop(actor);
    assert_eq!(actor_ref.try_send(1usize), Err(SendError));
}

async fn relay_msgs<M>(_: actor::Context<M, ThreadLocal>, relay_ref: ActorRef<M>, msgs: Vec<M>)
where
    M: Eq + fmt::Debug + Unpin,
{
    for msg in msgs {
        relay_ref.send(msg).await.unwrap()
    }
}

#[test]
fn send() {
    let expect_msgs = actor_fn(expect_msgs);
    let (actor, actor_ref) = init_local_actor(expect_msgs, MSGS.to_vec()).unwrap();
    let mut actor = Box::pin(actor);

    let relay_msgs = actor_fn(relay_msgs);
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
fn send_full_inbox() {
    let expected: Vec<usize> = (0..INBOX_SIZE + 2).collect();
    let expect_msgs = actor_fn(expect_msgs);
    let (actor, actor_ref) = init_local_actor(expect_msgs, expected.clone()).unwrap();
    let mut actor = Box::pin(actor);

    let relay_msgs = actor_fn(relay_msgs);
    let (relay_actor, _) = init_local_actor(relay_msgs, (actor_ref, expected)).unwrap();
    let mut relay_actor = Box::pin(relay_actor);

    // Fill the inbox.
    assert_eq!(poll_actor(Pin::as_mut(&mut relay_actor)), Poll::Pending);
    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Pending);

    // The last messages.
    assert_eq!(
        poll_actor(Pin::as_mut(&mut relay_actor)),
        Poll::Ready(Ok(()))
    );
    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
}

async fn relay_error(_: actor::Context<!, ThreadLocal>, relay_ref: ActorRef<usize>) {
    assert_eq!(relay_ref.send(1usize).await, Err(SendError));
}

#[test]
fn send_disconnected() {
    let expect_msgs = actor_fn(expect_msgs);
    let (actor, actor_ref) = init_local_actor(expect_msgs, Vec::new()).unwrap();
    drop(actor);

    let relay_error = actor_fn(relay_error);
    let (relay_actor, _) = init_local_actor(relay_error, actor_ref).unwrap();
    let mut relay_actor = Box::pin(relay_actor);

    assert_eq!(
        poll_actor(Pin::as_mut(&mut relay_actor)),
        Poll::Ready(Ok(()))
    );
}

#[test]
fn cloned() {
    let expected: Vec<usize> = (0..INBOX_SIZE - 1).collect();
    let expect_msgs = actor_fn(expect_msgs);
    let (actor, actor_ref) = init_local_actor(expect_msgs, expected.clone()).unwrap();
    let mut actor = Box::pin(actor);

    let m: Vec<_> = expected
        .into_iter()
        .map(|msg| (actor_ref.clone(), msg))
        .collect();
    for (actor_ref, msg) in m {
        actor_ref.try_send(msg).unwrap();
    }

    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
}

#[test]
fn mapped() {
    let expect_msgs = actor_fn(expect_msgs);
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
fn mapped_same_type() {
    let expect_msgs = actor_fn(expect_msgs);
    let expected = MSGS.iter().map(|s| (*s).to_owned()).collect();
    let (actor, actor_ref): (_, ActorRef<String>) =
        init_local_actor(expect_msgs, expected).unwrap();
    let mut actor = Box::pin(actor);

    // Same as the original `ActorRef`.
    let actor_ref: ActorRef<String> = actor_ref.map();
    for msg in MSGS {
        actor_ref.try_send(*msg).unwrap();
    }

    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
}

#[test]
fn mapped_send() {
    let expect_msgs = actor_fn(expect_msgs);
    let expected = MSGS.iter().map(|s| (*s).to_owned()).collect();
    let (actor, actor_ref) = init_local_actor(expect_msgs, expected).unwrap();
    let mut actor = Box::pin(actor);
    let actor_ref: ActorRef<&str> = actor_ref.map();

    let relay_msgs = actor_fn(relay_msgs);
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
fn mapped_cloned() {
    let expected: Vec<usize> = (0..INBOX_SIZE - 1).collect();
    let expect_msgs = actor_fn(expect_msgs);
    let (actor, actor_ref) = init_local_actor(expect_msgs, expected.clone()).unwrap();
    let actor_ref = actor_ref.map();
    let mut actor = Box::pin(actor);

    let m: Vec<(ActorRef<u8>, u8)> = expected
        .into_iter()
        .map(|msg| (actor_ref.clone(), msg as u8))
        .collect();
    for (actor_ref, msg) in m {
        actor_ref.try_send(msg).unwrap();
    }

    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
}

#[test]
fn try_mapped() {
    let expect_msgs = actor_fn(expect_msgs);
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

#[test]
fn try_mapped_same_type() {
    let expect_msgs = actor_fn(expect_msgs);
    let expected = vec![
        NonZeroUsize::new(1).unwrap(),
        NonZeroUsize::new(2).unwrap(),
        NonZeroUsize::new(3).unwrap(),
    ];
    let (actor, actor_ref): (_, ActorRef<NonZeroUsize>) =
        init_local_actor(expect_msgs, expected).unwrap();
    let mut actor = Box::pin(actor);

    // NOTE: same type as the original actor ref.
    let actor_ref: ActorRef<NonZeroUsize> = actor_ref.try_map();
    for n in 1..4usize {
        let msg = NonZeroUsize::new(n).unwrap();
        actor_ref.try_send(msg).unwrap();
    }

    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
}

#[test]
fn try_mapped_send() {
    let expect_msgs = actor_fn(expect_msgs);
    let expected = MSGS.iter().map(|s| (*s).to_owned()).collect();
    let (actor, actor_ref) = init_local_actor(expect_msgs, expected).unwrap();
    let mut actor = Box::pin(actor);
    let actor_ref: ActorRef<&str> = actor_ref.try_map();

    let relay_msgs = actor_fn(relay_msgs);
    let (relay_actor, _) = init_local_actor(relay_msgs, (actor_ref, MSGS.to_vec())).unwrap();
    let mut relay_actor = Box::pin(relay_actor);

    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Pending);
    assert_eq!(
        poll_actor(Pin::as_mut(&mut relay_actor)),
        Poll::Ready(Ok(()))
    );
    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
}

// Sends zero to a `NonZeroUsize` mapped actor reference causing a conversion
// error.
async fn send_error(_: actor::Context<!, ThreadLocal>, relay_ref: ActorRef<usize>) {
    let res = relay_ref.send(0usize).await;
    assert_eq!(res, Err(SendError));
}

#[test]
fn try_mapped_send_conversion_error() {
    let expect_msgs = actor_fn(expect_msgs);
    let expected = vec![NonZeroUsize::new(1).unwrap(), NonZeroUsize::new(2).unwrap()];
    let (actor, actor_ref) = init_local_actor(expect_msgs, expected.clone()).unwrap();
    let mut actor = Box::pin(actor);
    let actor_ref: ActorRef<usize> = actor_ref.try_map();

    let send_error = actor_fn(send_error);
    let (relay_actor, _) = init_local_actor(send_error, actor_ref.clone()).unwrap();
    let mut relay_actor = Box::pin(relay_actor);

    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Pending);
    assert_eq!(
        poll_actor(Pin::as_mut(&mut relay_actor)),
        Poll::Ready(Ok(()))
    );
    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Pending);

    for msg in expected {
        actor_ref.try_send(msg).unwrap();
    }

    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
}

#[test]
fn try_mapped_cloned() {
    let expected: Vec<usize> = (0..INBOX_SIZE - 1).collect();
    let expect_msgs = actor_fn(expect_msgs);
    let (actor, actor_ref) = init_local_actor(expect_msgs, expected.clone()).unwrap();
    let actor_ref = actor_ref.try_map();
    let mut actor = Box::pin(actor);

    let m: Vec<(ActorRef<u8>, u8)> = expected
        .into_iter()
        .map(|msg| (actor_ref.clone(), msg as u8))
        .collect();
    for (actor_ref, msg) in m {
        actor_ref.try_send(msg).unwrap();
    }

    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
}

#[test]
fn is_connected() {
    let expect_msgs = actor_fn::<_, Vec<()>, _, _, _>(expect_msgs);
    let (actor, actor_ref) = init_local_actor(expect_msgs, Vec::new()).unwrap();
    assert!(actor_ref.is_connected());

    drop(actor);
    assert!(!actor_ref.is_connected());
}

#[test]
fn mapped_is_connected() {
    let expect_msgs = actor_fn::<_, u8, _, _, _>(expect_msgs);
    let (actor, actor_ref) = init_local_actor(expect_msgs, Vec::new()).unwrap();
    let actor_ref: ActorRef<u8> = actor_ref.map();
    assert!(actor_ref.is_connected());

    drop(actor);
    assert!(!actor_ref.is_connected());
}

#[test]
fn mapped_fn() {
    let expect_msgs = actor_fn(expect_msgs);
    let expected = MSGS.iter().map(|s| (*s).to_owned()).collect();
    let (actor, actor_ref): (_, ActorRef<String>) =
        init_local_actor(expect_msgs, expected).unwrap();
    let mut actor = Box::pin(actor);

    let actor_ref: ActorRef<&str> = actor_ref.map_fn(|msg: &str| msg.to_owned());
    for msg in MSGS {
        actor_ref.try_send(*msg).unwrap();
    }

    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
}

#[test]
fn mapped_fn_send() {
    let expect_msgs = actor_fn(expect_msgs);
    let expected = MSGS.iter().map(|s| (*s).to_owned()).collect();
    let (actor, actor_ref) = init_local_actor(expect_msgs, expected).unwrap();
    let mut actor = Box::pin(actor);
    let actor_ref: ActorRef<&str> = actor_ref.map_fn(|msg: &str| msg.to_owned());

    let relay_msgs = actor_fn(relay_msgs);
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
fn mapped_fn_cloned() {
    let expected: Vec<usize> = (0..INBOX_SIZE - 1).collect();
    let expect_msgs = actor_fn(expect_msgs);
    let (actor, actor_ref) = init_local_actor(expect_msgs, expected.clone()).unwrap();
    let actor_ref = actor_ref.map_fn(|msg: u8| msg as usize);
    let mut actor = Box::pin(actor);

    let m: Vec<(ActorRef<u8>, u8)> = expected
        .into_iter()
        .map(|msg| (actor_ref.clone(), msg as u8))
        .collect();
    for (actor_ref, msg) in m {
        actor_ref.try_send(msg).unwrap();
    }

    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
}

#[test]
fn try_mapped_fn() {
    let expect_msgs = actor_fn(expect_msgs);
    let expected = vec![
        NonZeroUsize::new(1).unwrap(),
        NonZeroUsize::new(2).unwrap(),
        NonZeroUsize::new(3).unwrap(),
    ];
    let (actor, actor_ref): (_, ActorRef<NonZeroUsize>) =
        init_local_actor(expect_msgs, expected).unwrap();
    let mut actor = Box::pin(actor);

    let actor_ref: ActorRef<usize> =
        actor_ref.try_map_fn(|msg| NonZeroUsize::new(msg).ok_or(SendError));
    assert!(actor_ref.try_send(0usize).is_err());
    for msg in 1..4usize {
        actor_ref.try_send(msg).unwrap();
    }

    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
}

#[test]
fn try_mapped_fn_send() {
    let expect_msgs = actor_fn(expect_msgs);
    let expected = MSGS.iter().map(|s| (*s).to_owned()).collect();
    let (actor, actor_ref) = init_local_actor(expect_msgs, expected).unwrap();
    let mut actor = Box::pin(actor);
    let actor_ref: ActorRef<&str> = actor_ref.try_map_fn::<_, _, !>(|msg: &str| Ok(msg.to_owned()));

    let relay_msgs = actor_fn(relay_msgs);
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
fn try_mapped_fn_send_conversion_error() {
    let expect_msgs = actor_fn(expect_msgs);
    let expected = vec![NonZeroUsize::new(1).unwrap(), NonZeroUsize::new(2).unwrap()];
    let (actor, actor_ref) = init_local_actor(expect_msgs, expected.clone()).unwrap();
    let mut actor = Box::pin(actor);
    let actor_ref: ActorRef<usize> =
        actor_ref.try_map_fn(|msg| NonZeroUsize::new(msg).ok_or(SendError));

    let send_error = actor_fn(send_error);
    let (relay_actor, _) = init_local_actor(send_error, actor_ref.clone()).unwrap();
    let mut relay_actor = Box::pin(relay_actor);

    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Pending);
    assert_eq!(
        poll_actor(Pin::as_mut(&mut relay_actor)),
        Poll::Ready(Ok(()))
    );
    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Pending);

    for msg in expected {
        actor_ref.try_send(msg).unwrap();
    }

    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
}

#[test]
fn try_mapped_fn_cloned() {
    let expected: Vec<usize> = (0..INBOX_SIZE - 1).collect();
    let expect_msgs = actor_fn(expect_msgs);
    let (actor, actor_ref) = init_local_actor(expect_msgs, expected.clone()).unwrap();
    let actor_ref = actor_ref.try_map_fn::<_, _, !>(|msg| Ok(msg as usize));
    let mut actor = Box::pin(actor);

    let m: Vec<(ActorRef<u8>, u8)> = expected
        .into_iter()
        .map(|msg| (actor_ref.clone(), msg as u8))
        .collect();
    for (actor_ref, msg) in m {
        actor_ref.try_send(msg).unwrap();
    }

    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
}

#[test]
fn mapped_fn_is_connected() {
    let expect_msgs = actor_fn(expect_msgs);
    let (actor, actor_ref) = init_local_actor(expect_msgs, Vec::new()).unwrap();
    let actor_ref: ActorRef<u8> = actor_ref.map_fn(|msg| msg as usize);
    assert!(actor_ref.is_connected());

    drop(actor);
    assert!(!actor_ref.is_connected());
}

#[test]
fn sends_to() {
    let expect_msgs = actor_fn(expect_msgs);
    let (_, actor_ref1a) = init_local_actor(expect_msgs, Vec::new()).unwrap();
    let actor_ref1b: ActorRef<u16> = actor_ref1a.clone();
    let actor_ref1c: ActorRef<u8> = actor_ref1a.clone().map();
    let (_, actor_ref2a) = init_local_actor(expect_msgs, Vec::new()).unwrap();
    let actor_ref2b = actor_ref2a.clone();
    let actor_ref2c: ActorRef<u8> = actor_ref2a.clone().map();

    assert!(actor_ref1a.sends_to(&actor_ref1a));
    assert!(actor_ref1a.sends_to(&actor_ref1b));
    assert!(actor_ref1a.sends_to(&actor_ref1c));
    assert!(actor_ref2a.sends_to(&actor_ref2a));
    assert!(actor_ref2a.sends_to(&actor_ref2b));
    assert!(actor_ref2a.sends_to(&actor_ref2c));

    assert!(!actor_ref1a.sends_to(&actor_ref2a));
    assert!(!actor_ref1a.sends_to(&actor_ref2b));
    assert!(!actor_ref1a.sends_to(&actor_ref2c));
    assert!(!actor_ref2a.sends_to(&actor_ref1a));
    assert!(!actor_ref2a.sends_to(&actor_ref1b));
    assert!(!actor_ref2a.sends_to(&actor_ref1c));
}

#[test]
fn send_error_format() {
    assert_eq!(format!("{}", SendError), "unable to send message");
}

async fn wake_on_send(_: actor::Context<usize, ThreadLocal>, relay_ref: ActorRef<usize>) {
    relay_ref
        .send(123usize)
        .await
        .expect("failed to send message");
}

async fn wake_on_receive(mut ctx: actor::Context<usize, ThreadLocal>, expected: Vec<usize>) {
    for expected in expected {
        let got = ctx.receive_next().await.expect("missing message");
        assert_eq!(got, expected);
    }
}

#[test]
fn waking() {
    let mut runtime = Runtime::setup().num_threads(1).build().unwrap();
    runtime
        .run_on_workers::<_, !>(|mut runtime_ref| {
            let mut expected: Vec<usize> = (0..INBOX_SIZE).into_iter().collect();
            expected.push(123);
            let wake_on_receive = actor_fn(wake_on_receive);
            let options = ActorOptions::default().with_priority(Priority::LOW);
            let actor_ref =
                runtime_ref.spawn_local(NoSupervisor, wake_on_receive, expected, options);

            // Fill the inbox.
            for msg in 0..INBOX_SIZE {
                actor_ref.try_send(msg).unwrap();
            }

            let wake_on_send = actor_fn(wake_on_send);
            // Run before `wake_on_receive`.
            let options = ActorOptions::default().with_priority(Priority::HIGH);
            let _ = runtime_ref.spawn_local(NoSupervisor, wake_on_send, actor_ref, options);

            // Once we start the runtime:
            // 1. `wake_on_send` to call `ActorRef::send` and return pending.
            // 2. `wake_on_receive` should empty the inbox.
            // 3. `wake_on_send` should be waken again and send the last message.
            // 4. `wake_on_receive` should receive the last message.

            Ok(())
        })
        .unwrap();
    runtime.start().unwrap();
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
struct Ping;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
struct Pong;

async fn ping(_: actor::Context<!, ThreadLocal>, relay_ref: ActorRef<RpcTestMessage>) {
    let rpc = relay_ref.rpc(Ping);
    let res = rpc.await;
    assert_eq!(res, Ok(Pong));
}

enum RpcTestMessage {
    Ping(RpcMessage<Ping, Pong>),
    Check,
}

impl From<RpcMessage<Ping, Pong>> for RpcTestMessage {
    fn from(msg: RpcMessage<Ping, Pong>) -> RpcTestMessage {
        RpcTestMessage::Ping(msg)
    }
}

async fn pong(mut ctx: actor::Context<RpcTestMessage, ThreadLocal>) {
    while let Ok(msg) = ctx.receive_next().await {
        match msg {
            RpcTestMessage::Ping(msg) => msg.handle(|_| ready(Pong)).await.unwrap(),
            RpcTestMessage::Check => {}
        }
    }
}

#[test]
fn rpc() {
    let pong = actor_fn(pong);
    let (pong_actor, relay_ref) = init_local_actor(pong, ()).unwrap();
    let mut pong_actor = Box::pin(pong_actor);

    let ping = actor_fn(ping);
    let (ping_actor, _) = init_local_actor(ping, relay_ref).unwrap();
    let mut ping_actor = Box::pin(ping_actor);

    // Send RPC requests.
    assert_eq!(poll_actor(Pin::as_mut(&mut ping_actor)), Poll::Pending);
    // Return response.
    assert_eq!(poll_actor(Pin::as_mut(&mut pong_actor)), Poll::Pending);

    // Handle response.
    assert_eq!(
        poll_actor(Pin::as_mut(&mut ping_actor)),
        Poll::Ready(Ok(()))
    );
    // All actor references dropped.
    assert_eq!(
        poll_actor(Pin::as_mut(&mut pong_actor)),
        Poll::Ready(Ok(()))
    );
}

async fn rpc_send_error_actor(
    _: actor::Context<!, ThreadLocal>,
    relay_ref: ActorRef<RpcTestMessage>,
) {
    let rpc = relay_ref.rpc(Ping);
    let res = rpc.await;
    assert_eq!(res, Err(RpcError::SendError));
}

#[test]
fn rpc_send_error() {
    let pong = actor_fn(pong);
    let (pong_actor, relay_ref) = init_local_actor(pong, ()).unwrap();
    drop(pong_actor);

    let actor = actor_fn(rpc_send_error_actor);
    let (send_error, _) = init_local_actor(actor, relay_ref).unwrap();
    let mut send_error = Box::pin(send_error);

    // Send RPC requests.
    assert_eq!(
        poll_actor(Pin::as_mut(&mut send_error)),
        Poll::Ready(Ok(()))
    );
}

#[test]
fn rpc_full_inbox() {
    let pong = actor_fn(pong);
    let (pong_actor, relay_ref) = init_local_actor(pong, ()).unwrap();
    let mut pong_actor = Box::pin(pong_actor);

    for _ in 0..INBOX_SIZE {
        relay_ref.try_send(RpcTestMessage::Check).unwrap();
    }

    let ping = actor_fn(ping);
    let (ping_actor, _) = init_local_actor(ping, relay_ref).unwrap();
    let mut ping_actor = Box::pin(ping_actor);

    // Can't send message yet, inbox full.
    assert_eq!(poll_actor(Pin::as_mut(&mut ping_actor)), Poll::Pending);
    // Handle all messages send above.
    assert_eq!(poll_actor(Pin::as_mut(&mut pong_actor)), Poll::Pending);

    // Send RPC requests.
    assert_eq!(poll_actor(Pin::as_mut(&mut ping_actor)), Poll::Pending);
    // Return response.
    assert_eq!(poll_actor(Pin::as_mut(&mut pong_actor)), Poll::Pending);

    // Handle response.
    assert_eq!(
        poll_actor(Pin::as_mut(&mut ping_actor)),
        Poll::Ready(Ok(()))
    );
    // All actor references dropped.
    assert_eq!(
        poll_actor(Pin::as_mut(&mut pong_actor)),
        Poll::Ready(Ok(()))
    );
}

async fn ping_no_response(_: actor::Context<!, ThreadLocal>, relay_ref: ActorRef<RpcTestMessage>) {
    let rpc = relay_ref.rpc(Ping);
    let res = rpc.await;
    assert_eq!(res, Err(RpcError::NoResponse));
}

#[test]
fn rpc_no_response() {
    let pong = actor_fn(pong);
    let (pong_actor, relay_ref) = init_local_actor(pong, ()).unwrap();

    let ping = actor_fn(ping_no_response);
    let (ping_actor, _) = init_local_actor(ping, relay_ref).unwrap();
    let mut ping_actor = Box::pin(ping_actor);

    // Send RPC request.
    assert_eq!(poll_actor(Pin::as_mut(&mut ping_actor)), Poll::Pending);
    // Drop the responding actor.
    drop(pong_actor);
    // Expect no response.
    assert_eq!(
        poll_actor(Pin::as_mut(&mut ping_actor)),
        Poll::Ready(Ok(()))
    );
}

async fn pong_respond_error(mut ctx: actor::Context<RpcTestMessage, ThreadLocal>) {
    while let Ok(msg) = ctx.receive_next().await {
        match msg {
            RpcTestMessage::Ping(RpcMessage { response, .. }) => {
                assert_eq!(response.respond(Pong), Err(SendError));
            }
            RpcTestMessage::Check => {}
        }
    }
}

#[test]
fn rpc_respond_error() {
    let pong = actor_fn(pong_respond_error);
    let (pong_actor, relay_ref) = init_local_actor(pong, ()).unwrap();
    let mut pong_actor = Box::pin(pong_actor);

    let ping = actor_fn(ping);
    let (ping_actor, _) = init_local_actor(ping, relay_ref).unwrap();
    let mut ping_actor = Box::pin(ping_actor);

    // Send RPC requests.
    assert_eq!(poll_actor(Pin::as_mut(&mut ping_actor)), Poll::Pending);
    // Drop the receiving actor so we can't respond.
    drop(ping_actor);
    // All actor references dropped.
    assert_eq!(
        poll_actor(Pin::as_mut(&mut pong_actor)),
        Poll::Ready(Ok(()))
    );
}

async fn pong_is_connected(mut ctx: actor::Context<RpcTestMessage, ThreadLocal>) {
    while let Ok(msg) = ctx.receive_next().await {
        match msg {
            RpcTestMessage::Ping(RpcMessage { response, .. }) => {
                response.is_connected();

                pending_once().await;

                assert!(!response.is_connected());
                assert_eq!(response.respond(Pong), Err(SendError));
            }
            RpcTestMessage::Check => {}
        }
    }
}

#[test]
fn rpc_response_is_connected() {
    let pong = actor_fn(pong_is_connected);
    let (pong_actor, relay_ref) = init_local_actor(pong, ()).unwrap();
    let mut pong_actor = Box::pin(pong_actor);

    let ping = actor_fn(ping);
    let (ping_actor, _) = init_local_actor(ping, relay_ref).unwrap();
    let mut ping_actor = Box::pin(ping_actor);

    // Send RPC requests.
    assert_eq!(poll_actor(Pin::as_mut(&mut ping_actor)), Poll::Pending);
    // First check.
    assert_eq!(poll_actor(Pin::as_mut(&mut pong_actor)), Poll::Pending);
    // Cause `is_connected` to return false.
    drop(ping_actor);
    assert_eq!(
        poll_actor(Pin::as_mut(&mut pong_actor)),
        Poll::Ready(Ok(()))
    );
}

#[test]
fn rpc_error_format() {
    assert_eq!(format!("{}", RpcError::SendError), "unable to send message");
    assert_eq!(format!("{}", RpcError::SendError), format!("{}", SendError));
    assert_eq!(format!("{}", RpcError::NoResponse), "no RPC response");
}

async fn wake_on_response(_: actor::Context<!, ThreadLocal>, relay_ref: ActorRef<RpcTestMessage>) {
    let rpc = relay_ref.rpc(Ping);
    let res = rpc.await;
    assert_eq!(res, Ok(Pong));
}

async fn wake_on_rpc_receive(mut ctx: actor::Context<RpcTestMessage, ThreadLocal>) {
    while let Ok(msg) = ctx.receive_next().await {
        match msg {
            RpcTestMessage::Ping(msg) => msg.handle(|_| ready(Pong)).await.unwrap(),
            RpcTestMessage::Check => {}
        }
    }
}

#[test]
fn rpc_waking() {
    let mut runtime = Runtime::setup().num_threads(1).build().unwrap();
    runtime
        .run_on_workers::<_, !>(|mut runtime_ref| {
            let wake_on_rpc_receive = actor_fn(wake_on_rpc_receive);
            let options = ActorOptions::default().with_priority(Priority::LOW);
            let actor_ref = runtime_ref.spawn_local(NoSupervisor, wake_on_rpc_receive, (), options);

            // Fill the inbox.
            for _ in 0..INBOX_SIZE {
                actor_ref.try_send(RpcTestMessage::Check).unwrap();
            }

            let wake_on_response = actor_fn(wake_on_response);
            // Run before `wake_on_rpc_receive`.
            let options = ActorOptions::default().with_priority(Priority::HIGH);
            let _ = runtime_ref.spawn_local(NoSupervisor, wake_on_response, actor_ref, options);

            // Once we start the runtime:
            // 1. `wake_on_response` calls `ActorRef::rpc` and returns pending.
            // 2. `wake_on_rpc_receive` should empty the inbox.
            // 3. `wake_on_response` should be woken again and send the RPC request.
            // 4. `wake_on_receive` should receive the RPC request and respond.
            // 5. `wake_on_response` should receive the response and return `Ok`.
            // 6. `wake_on_receive` should return `Ok`.

            Ok(())
        })
        .unwrap();
    runtime.start().unwrap();
}

async fn stop_on_run(ctx: actor::Context<Infallible, ThreadLocal>) {
    drop(ctx);
}

#[test]
fn join() {
    let stop_on_run = actor_fn(stop_on_run);
    let (actor, actor_ref) = init_local_actor(stop_on_run, ()).unwrap();
    let mut actor = Box::pin(actor);

    let future = actor_ref.join();
    let mut future = Box::pin(future);

    assert_eq!(poll_future(Pin::new(&mut future)), Poll::Pending);

    assert_eq!(poll_actor(Pin::new(&mut actor)), Poll::Ready(Ok(())));
    assert_eq!(poll_future(Pin::new(&mut future)), Poll::Ready(()));
}

#[test]
fn join_mapped() {
    let stop_on_run = actor_fn(stop_on_run);
    let (actor, actor_ref) = init_local_actor(stop_on_run, ()).unwrap();
    let mut actor = Box::pin(actor);

    let actor_ref = actor_ref.map::<!>();
    let future = actor_ref.join();
    let mut future = Box::pin(future);

    assert_eq!(poll_future(Pin::new(&mut future)), Poll::Pending);

    assert_eq!(poll_actor(Pin::new(&mut actor)), Poll::Ready(Ok(())));
    assert_eq!(poll_future(Pin::new(&mut future)), Poll::Ready(()));
}

#[test]
fn join_before_actor_finished() {
    let stop_on_run = actor_fn(stop_on_run);
    let (actor, actor_ref) = init_local_actor(stop_on_run, ()).unwrap();
    let mut actor = Box::pin(actor);

    assert_eq!(poll_actor(Pin::new(&mut actor)), Poll::Ready(Ok(())));

    let future = actor_ref.join();
    let mut future = Box::pin(future);

    assert_eq!(poll_future(Pin::new(&mut future)), Poll::Ready(()));
}

#[derive(Debug)]
enum CalcMessage {
    Get(RpcMessage<(), usize>),
    Add(RpcMessage<usize, ()>),
    Add2(RpcMessage<(usize, usize), ()>),
}

from_message!(CalcMessage::Get(()) -> usize);
from_message!(CalcMessage::Add(usize) -> ());
from_message!(CalcMessage::Add2((usize, usize)) -> ());

#[derive(Debug, Eq, PartialEq)]
struct Overflow;

async fn calc_actor(mut ctx: actor::Context<CalcMessage, ThreadLocal>) -> Result<(), Overflow> {
    let mut count: usize = 10;
    while let Ok(msg) = ctx.receive_next().await {
        match msg {
            CalcMessage::Get(msg) => msg.handle(|()| async { count }).await.unwrap(),
            CalcMessage::Add(msg) => {
                let c = &mut count;
                msg.handle(|amount| async move { *c += amount })
                    .await
                    .unwrap()
            }
            CalcMessage::Add2(msg) => {
                let c = &mut count;
                msg.try_handle(|(a, b)| async move {
                    *c = c.checked_add(a).ok_or(Overflow)?;
                    *c = c.checked_add(b).ok_or(Overflow)?;
                    Ok(())
                })
                .await?
                .unwrap()
            }
        }
    }
    Ok(())
}

#[test]
fn rpc_message_handle() {
    let calc_actor = actor_fn(calc_actor);
    let (actor, actor_ref) = init_local_actor(calc_actor, ()).unwrap();
    let mut actor = pin!(actor);

    let mut add_rpc = actor_ref.rpc(123);
    assert_eq!(poll_future(Pin::new(&mut add_rpc)), Poll::Pending);
    let mut get_rpc = actor_ref.rpc(());
    assert_eq!(poll_future(Pin::new(&mut get_rpc)), Poll::Pending);

    assert_eq!(poll_future(Pin::new(&mut actor)), Poll::Pending);

    assert_eq!(poll_future(Pin::new(&mut add_rpc)), Poll::Ready(Ok(())));
    drop(add_rpc);
    assert_eq!(poll_future(Pin::new(&mut get_rpc)), Poll::Ready(Ok(133)));
    drop(get_rpc);

    drop(actor_ref);
    assert_eq!(poll_future(Pin::new(&mut actor)), Poll::Ready(Ok(())));
}

#[test]
fn rpc_message_handle_skip_if_no_receiver() {
    let calc_actor = actor_fn(calc_actor);
    let (actor, actor_ref) = init_local_actor(calc_actor, ()).unwrap();
    let mut actor = pin!(actor);

    let mut add_rpc = actor_ref.rpc(123);
    // Make sure the value is send.
    assert_eq!(poll_future(Pin::new(&mut add_rpc)), Poll::Pending);
    drop(add_rpc);

    let mut get_rpc = actor_ref.rpc(());
    assert_eq!(poll_future(Pin::new(&mut get_rpc)), Poll::Pending);

    assert_eq!(poll_future(Pin::new(&mut actor)), Poll::Pending);

    assert_eq!(poll_future(Pin::new(&mut get_rpc)), Poll::Ready(Ok(10)));
    drop(get_rpc);

    drop(actor_ref);
    assert_eq!(poll_future(Pin::new(&mut actor)), Poll::Ready(Ok(())));
}

#[test]
fn rpc_message_try_handle() {
    let calc_actor = actor_fn(calc_actor);
    let (actor, actor_ref) = init_local_actor(calc_actor, ()).unwrap();
    let mut actor = pin!(actor);

    let mut add_rpc = actor_ref.rpc((usize::MAX, usize::MAX));
    assert_eq!(poll_future(Pin::new(&mut add_rpc)), Poll::Pending);

    assert_eq!(
        poll_future(Pin::new(&mut actor)),
        Poll::Ready(Err(Overflow))
    );
}

#[test]
fn rpc_message_try_handle_skip_if_no_receiver() {
    let calc_actor = actor_fn(calc_actor);
    let (actor, actor_ref) = init_local_actor(calc_actor, ()).unwrap();
    let mut actor = pin!(actor);

    let mut add_rpc = actor_ref.rpc((usize::MAX, usize::MAX));
    // Make sure the value is send.
    assert_eq!(poll_future(Pin::new(&mut add_rpc)), Poll::Pending);
    drop(add_rpc);

    let mut get_rpc = actor_ref.rpc(());
    assert_eq!(poll_future(Pin::new(&mut get_rpc)), Poll::Pending);

    assert_eq!(poll_future(Pin::new(&mut actor)), Poll::Pending);

    assert_eq!(poll_future(Pin::new(&mut get_rpc)), Poll::Ready(Ok(10)));
    drop(get_rpc);

    drop(actor_ref);
    assert_eq!(poll_future(Pin::new(&mut actor)), Poll::Ready(Ok(())));
}
