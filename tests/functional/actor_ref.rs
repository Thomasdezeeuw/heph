//! Tests related to `ActorRef`.

#![cfg(feature = "test")]

use std::fmt;
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::task::Poll;

use heph::actor_ref::{ActorRef, RpcError, RpcMessage, SendError, SendValue};
use heph::rt::options::Priority;
use heph::supervisor::NoSupervisor;
use heph::test::{init_local_actor, poll_actor};
use heph::{actor, ActorOptions, Runtime};

use crate::util::{assert_send, assert_size, assert_sync, pending_once};

/// Default size of the inbox, keep in sync with the inbox crate.
const INBOX_SIZE: usize = 8;

const MSGS: &[&str] = &["Hello world", "Hello mars", "Hello moon"];

#[test]
fn size() {
    assert_size::<ActorRef<()>>(24);
    assert_size::<SendValue<'_, '_, ()>>(64);
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

async fn expect_msgs<M>(mut ctx: actor::Context<M>, expected: Vec<M>) -> Result<(), !>
where
    M: Eq + fmt::Debug,
{
    for expected in expected {
        let got = ctx.receive_next().await.expect("missing message");
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

#[test]
fn try_send_full_inbox() {
    let expected: Vec<usize> = (0..INBOX_SIZE + 2).collect();
    let expect_msgs = expect_msgs as fn(_, _) -> _;
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
    let expect_msgs = expect_msgs as fn(_, Vec<usize>) -> _;
    let (actor, actor_ref) = init_local_actor(expect_msgs, Vec::new()).unwrap();
    drop(actor);
    assert_eq!(actor_ref.try_send(1usize), Err(SendError));
}

async fn relay_msgs<M>(_: actor::Context<M>, relay_ref: ActorRef<M>, msgs: Vec<M>) -> Result<(), !>
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
fn send_full_inbox() {
    let expected: Vec<usize> = (0..INBOX_SIZE + 2).collect();
    let expect_msgs = expect_msgs as fn(_, _) -> _;
    let (actor, actor_ref) = init_local_actor(expect_msgs, expected.clone()).unwrap();
    let mut actor = Box::pin(actor);

    let relay_msgs = relay_msgs as fn(_, _, _) -> _;
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

async fn relay_error(_: actor::Context<!>, relay_ref: ActorRef<usize>) -> Result<(), !> {
    assert_eq!(relay_ref.send(1usize).await, Err(SendError));
    Ok(())
}

#[test]
fn send_disconnected() {
    let expect_msgs = expect_msgs as fn(_, _) -> _;
    let (actor, actor_ref) = init_local_actor(expect_msgs, Vec::new()).unwrap();
    drop(actor);

    let relay_error = relay_error as fn(_, _) -> _;
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
    let expect_msgs = expect_msgs as fn(_, _) -> _;
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
fn mapped_same_type() {
    let expect_msgs = expect_msgs as fn(_, _) -> _;
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
    let expect_msgs = expect_msgs as fn(_, _) -> _;
    let expected = MSGS.iter().map(|s| (*s).to_owned()).collect();
    let (actor, actor_ref) = init_local_actor(expect_msgs, expected).unwrap();
    let mut actor = Box::pin(actor);
    let actor_ref: ActorRef<&str> = actor_ref.map();

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
fn mapped_cloned() {
    let expected: Vec<usize> = (0..INBOX_SIZE - 1).collect();
    let expect_msgs = expect_msgs as fn(_, _) -> _;
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

#[test]
fn try_mapped_same_type() {
    let expect_msgs = expect_msgs as fn(_, _) -> _;
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
    let expect_msgs = expect_msgs as fn(_, _) -> _;
    let expected = MSGS.iter().map(|s| (*s).to_owned()).collect();
    let (actor, actor_ref) = init_local_actor(expect_msgs, expected).unwrap();
    let mut actor = Box::pin(actor);
    let actor_ref: ActorRef<&str> = actor_ref.try_map();

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

// Sends zero to a `NonZeroUsize` mapped actor reference causing a conversion
// error.
async fn send_error(_: actor::Context<!>, relay_ref: ActorRef<usize>) -> Result<(), !> {
    let res = relay_ref.send(0usize).await;
    assert_eq!(res, Err(SendError));
    Ok(())
}

#[test]
fn try_mapped_send_conversion_error() {
    let expect_msgs = expect_msgs as fn(_, _) -> _;
    let expected = vec![NonZeroUsize::new(1).unwrap(), NonZeroUsize::new(2).unwrap()];
    let (actor, actor_ref) = init_local_actor(expect_msgs, expected.clone()).unwrap();
    let mut actor = Box::pin(actor);
    let actor_ref: ActorRef<usize> = actor_ref.try_map();

    let send_error = send_error as fn(_, _) -> _;
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
    let expect_msgs = expect_msgs as fn(_, _) -> _;
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
    let expect_msgs = expect_msgs as fn(_, Vec<()>) -> _;
    let (actor, actor_ref) = init_local_actor(expect_msgs, Vec::new()).unwrap();
    assert!(actor_ref.is_connected());

    drop(actor);
    assert!(!actor_ref.is_connected());
}

#[test]
fn mapped_is_connected() {
    let expect_msgs = expect_msgs as fn(actor::Context<usize>, Vec<usize>) -> _;
    let (actor, actor_ref) = init_local_actor(expect_msgs, Vec::new()).unwrap();
    let actor_ref: ActorRef<u8> = actor_ref.map();
    assert!(actor_ref.is_connected());

    drop(actor);
    assert!(!actor_ref.is_connected());
}

#[test]
fn send_error_format() {
    assert_eq!(format!("{}", SendError), "unable to send message");
}

async fn wake_on_send(_: actor::Context<usize>, relay_ref: ActorRef<usize>) -> Result<(), !> {
    relay_ref
        .send(123usize)
        .await
        .expect("failed to send message");
    Ok(())
}

async fn wake_on_receive(mut ctx: actor::Context<usize>, expected: Vec<usize>) -> Result<(), !> {
    for expected in expected {
        let got = ctx.receive_next().await.expect("missing message");
        assert_eq!(got, expected);
    }
    Ok(())
}

#[test]
fn waking() {
    Runtime::new()
        .unwrap()
        .num_threads(1)
        .with_setup::<_, !>(|mut runtime_ref| {
            let mut expected: Vec<usize> = (0..INBOX_SIZE).into_iter().collect();
            expected.push(123);
            let wake_on_receive = wake_on_receive as fn(_, _) -> _;
            let options = ActorOptions::default().with_priority(Priority::LOW);
            let actor_ref =
                runtime_ref.spawn_local(NoSupervisor, wake_on_receive, expected, options);

            // Fill the inbox.
            for msg in 0..INBOX_SIZE {
                actor_ref.try_send(msg).unwrap();
            }

            let wake_on_send = wake_on_send as fn(_, _) -> _;
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
        .start()
        .unwrap();
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
struct Ping;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
struct Pong;

async fn ping(_: actor::Context<!>, relay_ref: ActorRef<RpcTestMessage>) -> Result<(), !> {
    let rpc = relay_ref.rpc(Ping);
    let res = rpc.await;
    assert_eq!(res, Ok(Pong));
    Ok(())
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

async fn pong(mut ctx: actor::Context<RpcTestMessage>) -> Result<(), !> {
    while let Ok(msg) = ctx.receive_next().await {
        match msg {
            RpcTestMessage::Ping(msg) => msg.handle(|_| Pong).unwrap(),
            RpcTestMessage::Check => {}
        }
    }
    Ok(())
}

#[test]
fn rpc() {
    let pong = pong as fn(_) -> _;
    let (pong_actor, relay_ref) = init_local_actor(pong, ()).unwrap();
    let mut pong_actor = Box::pin(pong_actor);

    let ping = ping as fn(_, _) -> _;
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
    _: actor::Context<!>,
    relay_ref: ActorRef<RpcTestMessage>,
) -> Result<(), !> {
    let rpc = relay_ref.rpc(Ping);
    let res = rpc.await;
    assert_eq!(res, Err(RpcError::SendError));
    Ok(())
}

#[test]
fn rpc_send_error() {
    let pong = pong as fn(_) -> _;
    let (pong_actor, relay_ref) = init_local_actor(pong, ()).unwrap();
    drop(pong_actor);

    let actor = rpc_send_error_actor as fn(_, _) -> _;
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
    let pong = pong as fn(_) -> _;
    let (pong_actor, relay_ref) = init_local_actor(pong, ()).unwrap();
    let mut pong_actor = Box::pin(pong_actor);

    for _ in 0..INBOX_SIZE {
        relay_ref.try_send(RpcTestMessage::Check).unwrap();
    }

    let ping = ping as fn(_, _) -> _;
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

async fn ping_no_response(
    _: actor::Context<!>,
    relay_ref: ActorRef<RpcTestMessage>,
) -> Result<(), !> {
    let rpc = relay_ref.rpc(Ping);
    let res = rpc.await;
    assert_eq!(res, Err(RpcError::NoResponse));
    Ok(())
}

#[test]
fn rpc_no_response() {
    let pong = pong as fn(_) -> _;
    let (pong_actor, relay_ref) = init_local_actor(pong, ()).unwrap();

    let ping = ping_no_response as fn(_, _) -> _;
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

async fn pong_respond_error(mut ctx: actor::Context<RpcTestMessage>) -> Result<(), !> {
    while let Ok(msg) = ctx.receive_next().await {
        match msg {
            RpcTestMessage::Ping(RpcMessage { response, .. }) => {
                assert_eq!(response.respond(Pong), Err(SendError));
            }
            RpcTestMessage::Check => {}
        }
    }
    Ok(())
}

#[test]
fn rpc_respond_error() {
    let pong = pong_respond_error as fn(_) -> _;
    let (pong_actor, relay_ref) = init_local_actor(pong, ()).unwrap();
    let mut pong_actor = Box::pin(pong_actor);

    let ping = ping as fn(_, _) -> _;
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

async fn pong_is_connected(mut ctx: actor::Context<RpcTestMessage>) -> Result<(), !> {
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
    Ok(())
}

#[test]
fn rpc_response_is_connected() {
    let pong = pong_is_connected as fn(_) -> _;
    let (pong_actor, relay_ref) = init_local_actor(pong, ()).unwrap();
    let mut pong_actor = Box::pin(pong_actor);

    let ping = ping as fn(_, _) -> _;
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

async fn wake_on_response(
    _: actor::Context<!>,
    relay_ref: ActorRef<RpcTestMessage>,
) -> Result<(), !> {
    let rpc = relay_ref.rpc(Ping);
    let res = rpc.await;
    assert_eq!(res, Ok(Pong));
    Ok(())
}

async fn wake_on_rpc_receive(mut ctx: actor::Context<RpcTestMessage>) -> Result<(), !> {
    while let Ok(msg) = ctx.receive_next().await {
        match msg {
            RpcTestMessage::Ping(msg) => msg.handle(|_| Pong).unwrap(),
            RpcTestMessage::Check => {}
        }
    }
    Ok(())
}

#[test]
fn rpc_waking() {
    Runtime::new()
        .unwrap()
        .num_threads(1)
        .with_setup::<_, !>(|mut runtime_ref| {
            let wake_on_rpc_receive = wake_on_rpc_receive as fn(_) -> _;
            let options = ActorOptions::default().with_priority(Priority::LOW);
            let actor_ref = runtime_ref.spawn_local(NoSupervisor, wake_on_rpc_receive, (), options);

            // Fill the inbox.
            for _ in 0..INBOX_SIZE {
                actor_ref.try_send(RpcTestMessage::Check).unwrap();
            }

            let wake_on_response = wake_on_response as fn(_, _) -> _;
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
        .start()
        .unwrap();
}
