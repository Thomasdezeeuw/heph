//! Tests for the `from_message!` macro.

#![cfg(feature = "test")]

use std::pin::Pin;
use std::task::Poll;

use heph::actor_ref::{ActorRef, RpcMessage};
use heph::test::{init_local_actor, poll_actor};
use heph::{actor, from_message};

#[derive(Debug)]
enum Message {
    Msg(String),
    Rpc(RpcMessage<String, usize>),
    Rpc2(RpcMessage<(String, usize), (usize, usize)>),
}

from_message!(Message::Msg(String));
from_message!(Message::Rpc(String) -> usize);
from_message!(Message::Rpc2(String, usize) -> (usize, usize));

#[test]
fn from_message() {
    let pong_actor = pong_actor as fn(_) -> _;
    let (pong_actor, actor_ref) = init_local_actor(pong_actor, ()).unwrap();
    let mut pong_actor = Box::pin(pong_actor);

    let ping_actor = ping_actor as fn(_, _) -> _;
    let (ping_actor, actor_ref) = init_local_actor(ping_actor, actor_ref).unwrap();
    drop(actor_ref);
    let mut ping_actor = Box::pin(ping_actor);

    // Waiting for the first message.
    assert_eq!(poll_actor(Pin::as_mut(&mut pong_actor)), Poll::Pending);
    // Wait for first RPC call.
    assert_eq!(poll_actor(Pin::as_mut(&mut ping_actor)), Poll::Pending);

    // Receives first message and first RPC, waits for second RPC.
    assert_eq!(poll_actor(Pin::as_mut(&mut pong_actor)), Poll::Pending);
    // Receives first RPC response, waits on second RPC.
    assert_eq!(poll_actor(Pin::as_mut(&mut ping_actor)), Poll::Pending);

    // Receives second RPC and is done.
    assert_eq!(
        poll_actor(Pin::as_mut(&mut pong_actor)),
        Poll::Ready(Ok(()))
    );
    // Receives second RPC response and is done.
    assert_eq!(
        poll_actor(Pin::as_mut(&mut ping_actor)),
        Poll::Ready(Ok(()))
    );
}

async fn ping_actor(_: actor::Context<!>, actor_ref: ActorRef<Message>) -> Result<(), !> {
    actor_ref.send("Hello!".to_owned()).await.unwrap();

    let response = actor_ref.rpc("Rpc".to_owned()).await.unwrap();
    assert_eq!(response, 0);

    let response = actor_ref.rpc(("Rpc2".to_owned(), 2)).await.unwrap();
    assert_eq!(response, (1, 2));

    Ok(())
}

async fn pong_actor(mut ctx: actor::Context<Message>) -> Result<(), !> {
    let msg = ctx.receive_next().await.unwrap();
    assert!(matches!(msg, Message::Msg(msg) if msg == "Hello!"));

    let mut count = 0;
    if let Ok(Message::Rpc(RpcMessage { request, response })) = ctx.receive_next().await {
        assert_eq!(request, "Rpc");
        response.respond(count).unwrap();
        count += 1;
    }

    if let Ok(Message::Rpc2(RpcMessage { request, response })) = ctx.receive_next().await {
        let (msg, cnt) = request;
        assert_eq!(msg, "Rpc2");
        response.respond((count, cnt)).unwrap();
        count += cnt;
    }

    assert_eq!(count, 3);
    Ok(())
}
