//! Tests for the `from_message!` macro.

use heph::actor_ref::{ActorRef, RpcMessage};
use heph::supervisor::NoSupervisor;
use heph::{actor, from_message, ActorOptions, Runtime};

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
    Runtime::new()
        .unwrap()
        .with_setup::<_, !>(|mut runtime_ref| {
            let pong_actor = pong_actor as fn(_) -> _;
            let actor_ref =
                runtime_ref.spawn_local(NoSupervisor, pong_actor, (), ActorOptions::default());

            let ping_actor = ping_actor as fn(_, _) -> _;
            runtime_ref.spawn_local(NoSupervisor, ping_actor, actor_ref, ActorOptions::default());
            Ok(())
        })
        .start()
        .unwrap()
}

async fn ping_actor(mut ctx: actor::Context<!>, actor_ref: ActorRef<Message>) -> Result<(), !> {
    actor_ref.try_send("Hello!".to_owned()).unwrap();

    let response = actor_ref
        .rpc(&mut ctx, "Rpc".to_owned())
        .unwrap()
        .await
        .unwrap();
    assert_eq!(response, 0);

    let response = actor_ref
        .rpc(&mut ctx, ("Rpc2".to_owned(), 2))
        .unwrap()
        .await
        .unwrap();
    assert_eq!(response, (1, 2));

    Ok(())
}

async fn pong_actor(mut ctx: actor::Context<Message>) -> Result<(), !> {
    let msg = ctx.receive_next().await;
    assert!(matches!(msg, Message::Msg(msg) if msg == "Hello!"));

    let mut count = 0;
    if let Message::Rpc(RpcMessage { request, response }) = ctx.receive_next().await {
        assert_eq!(request, "Rpc");
        response.respond(count).unwrap();
        count += 1;
    }

    if let Message::Rpc2(RpcMessage { request, response }) = ctx.receive_next().await {
        let (msg, cnt) = request;
        assert_eq!(msg, "Rpc2");
        response.respond((count, cnt)).unwrap();
        count += cnt;
    }

    assert_eq!(count, 3);
    Ok(())
}
