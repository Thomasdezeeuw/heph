#![feature(never_type)]

use std::fmt;

use heph::actor_ref::{ActorRef, RpcMessage};
use heph::supervisor::NoSupervisor;
use heph::{actor, ActorOptions, Runtime, RuntimeError, RuntimeRef};

fn main() -> Result<(), RuntimeError> {
    // Setup is much like example 1, see that example for more information.
    Runtime::new().with_setup(add_rpc_actor).start()
}

fn add_rpc_actor(mut runtime_ref: RuntimeRef) -> Result<(), !> {
    // See example 1 for information on how to spawn actors.
    let pong_actor = pong_actor as fn(_) -> _;
    let actor_ref = runtime_ref.spawn(NoSupervisor, pong_actor, (), ActorOptions::default());

    let ping_actor = ping_actor as fn(_, _) -> _;
    runtime_ref.spawn(
        NoSupervisor,
        ping_actor,
        actor_ref,
        ActorOptions::default().mark_ready(),
    );

    Ok(())
}

async fn ping_actor(
    mut ctx: actor::Context<!>,
    mut actor_ref: ActorRef<PongMessage>,
) -> Result<(), !> {
    // Send our RPC request.
    let rpc = actor_ref.rpc(&mut ctx, Ping).unwrap();

    // Await until we get a response.
    match rpc.await {
        Ok(response) => println!("Got a RPC response: {}", response),
        Err(no_response) => eprintln!("RPC request error: {}", no_response),
    }
    Ok(())
}

// Message type to support the ping-pong RPC call.
type PongMessage = RpcMessage<Ping, Pong>;

async fn pong_actor(mut ctx: actor::Context<PongMessage>) -> Result<(), !> {
    // Await a message, same as all other messages.
    let RpcMessage { request, response } = ctx.receive_next().await;

    // Handle the RPC request.
    println!("Got a RPC request: {}", request);

    // Next we respond to the request.
    if let Err(err) = response.respond(Pong) {
        eprintln!("failed to respond to RPC: {}", err);
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
