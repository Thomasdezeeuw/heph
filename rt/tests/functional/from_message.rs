//! Tests for the `from_message!` macro.

use std::time::Duration;

use heph::actor::{self, actor_fn};
use heph::actor_ref::{ActorRef, RpcMessage};
use heph::from_message;
use heph::supervisor::NoSupervisor;
use heph_rt::spawn::ActorOptions;
use heph_rt::test::{join, try_spawn_local};
use heph_rt::ThreadLocal;

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
    let pong_ref = try_spawn_local(NoSupervisor, pong_actor, (), ActorOptions::default()).unwrap();

    let ping_actor = actor_fn(ping_actor);
    let ping_ref = try_spawn_local(
        NoSupervisor,
        ping_actor,
        pong_ref.clone(),
        ActorOptions::default(),
    )
    .unwrap();

    join(&ping_ref, Duration::from_secs(1)).unwrap();
    join(&pong_ref, Duration::from_secs(1)).unwrap();
}

async fn ping_actor(_: actor::Context<!, ThreadLocal>, actor_ref: ActorRef<Message>) {
    actor_ref.send("Hello!".to_owned()).await.unwrap();

    let response = actor_ref.rpc("Rpc".to_owned()).await.unwrap();
    assert_eq!(response, 0);

    let response = actor_ref.rpc(("Rpc2".to_owned(), 2)).await.unwrap();
    assert_eq!(response, (1, 2));
}

async fn pong_actor(mut ctx: actor::Context<Message, ThreadLocal>) {
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
