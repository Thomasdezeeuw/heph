#![feature(never_type)]

use std::fmt;

use heph::actor_ref::{ActorRef, RpcMessage};
use heph::supervisor::NoSupervisor;
use heph::{actor, rt, ActorOptions, Runtime, RuntimeRef};

fn main() -> Result<(), rt::Error> {
    // Setup is much like example 1, see that example for more information.
    Runtime::new()?.with_setup(add_rpc_actor).start()
}

fn add_rpc_actor(mut runtime_ref: RuntimeRef) -> Result<(), !> {
    // See example 1 for information on how to spawn actors.
    let pong_actor = pong_actor as fn(_) -> _;
    let actor_ref = runtime_ref.spawn_local(NoSupervisor, pong_actor, (), ActorOptions::default());

    let ping_actor = ping_actor as fn(_, _) -> _;
    runtime_ref.spawn_local(NoSupervisor, ping_actor, actor_ref, ActorOptions::default());

    Ok(())
}

async fn ping_actor(_: actor::Context<!>, actor_ref: ActorRef<PongMessage>) -> Result<(), !> {
    // Make a Remote Procedure Call (RPC) and await the response.
    match actor_ref.rpc(Ping).await {
        Ok(response) => println!("Got a RPC response: {}", response),
        Err(err) => eprintln!("RPC request error: {}", err),
    }

    Ok(())
}

// Message type to support the ping-pong RPC.
type PongMessage = RpcMessage<Ping, Pong>;

async fn pong_actor(mut ctx: actor::Context<PongMessage>) -> Result<(), !> {
    // Await a message, same as all other messages.
    while let Ok(msg) = ctx.receive_next().await {
        // Next we respond to the request.
        let res = msg.handle(|request| {
            println!("Got a RPC request: {}", request);
            // Return a response.
            Pong
        });

        if let Err(err) = res {
            eprintln!("failed to respond to RPC: {}", err);
        }
    }

    Ok(())
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
