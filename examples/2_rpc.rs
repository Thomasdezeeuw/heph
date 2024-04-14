use std::fmt;

use heph::actor::{self, actor_fn};
use heph::actor_ref::{ActorRef, RpcMessage};
use heph::future::ActorFuture;
use heph::supervisor::NoSupervisor;

mod runtime; // Replace this with your favourite `Future` runtime.

fn main() {
    // See example 1 for the creation of `ActorFuture`s.
    let (pong_future, pong_ref) = ActorFuture::new(NoSupervisor, actor_fn(pong_actor), ()).unwrap();
    let (ping_future, _) = ActorFuture::new(NoSupervisor, actor_fn(ping_actor), pong_ref).unwrap();

    // We run ours futures on our runtime.
    runtime::block_on2(ping_future, pong_future);
}

async fn ping_actor(_: actor::Context<()>, actor_ref: ActorRef<PongMessage>) {
    // Make a Remote Procedure Call (RPC) and await the response.
    match actor_ref.rpc(Ping).await {
        Ok(response) => println!("Got a RPC response: {response}"),
        Err(err) => eprintln!("RPC request error: {err}"),
    }
}

// Message type to support the ping-pong RPC.
type PongMessage = RpcMessage<Ping, Pong>;

async fn pong_actor(mut ctx: actor::Context<PongMessage>) {
    // Await a message, same as all other messages.
    while let Ok(msg) = ctx.receive_next().await {
        // Next we respond to the request.
        let res = msg
            .handle(|request| async move {
                println!("Got a RPC request: {request}");
                // Return a response.
                Pong
            })
            .await;

        if let Err(err) = res {
            eprintln!("failed to respond to RPC: {err}");
        }
    }
}

struct Ping;

impl fmt::Display for Ping {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Ping")
    }
}

struct Pong;

impl fmt::Display for Pong {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Pong")
    }
}
