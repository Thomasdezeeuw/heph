//! Tests for the `from_message!` macro.

use std::pin::pin;

use heph::actor::{self, actor_fn};
use heph::actor_ref::{ActorRef, RpcMessage};
use heph::supervisor::NoSupervisor;
use heph::{ActorFuture, from_message};

use crate::util::block_on_many;

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
    let pong_actor = actor_fn(pong_actor);
    let (pong_actor, pong_ref) = ActorFuture::new(NoSupervisor, pong_actor, ());

    let ping_actor = actor_fn(ping_actor);
    let (ping_actor, _) = ActorFuture::new(NoSupervisor, ping_actor, pong_ref.clone());

    block_on_many(vec![pin!(ping_actor), pin!(pong_actor)]);
}

async fn ping_actor(_: actor::Context<!>, actor_ref: ActorRef<Message>) {
    actor_ref.send("Hello!".to_owned()).await.unwrap();

    let response = actor_ref.rpc("Rpc".to_owned()).await.unwrap();
    assert_eq!(response, 0);

    let response = actor_ref.rpc(("Rpc2".to_owned(), 2)).await.unwrap();
    assert_eq!(response, (1, 2));
}

async fn pong_actor(mut ctx: actor::Context<Message>) {
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
}
