#![feature(never_type)]

use std::io;

use heph::actor::context::ThreadSafe;
use heph::net::relay::{Relay, RemoteRelay};
use heph::supervisor::NoSupervisor;
use heph::{actor, rt, ActorOptions, Runtime};
use log::info;

fn main() -> Result<(), rt::Error<io::Error>> {
    heph::log::init();

    let local_address = "127.0.0.1:9001".parse().unwrap();

    let mut runtime = Runtime::new().map_err(rt::Error::map_type)?;

    // Start our greeter actor.
    #[allow(trivial_casts)]
    let greeter_actor = greeter_actor as fn(_) -> _;
    let options = ActorOptions::default().mark_ready();
    let local_actor_ref = runtime.spawn(NoSupervisor, greeter_actor, (), options);

    // Create a router that relays all remote messages to our local actor.
    let router = Relay::to(local_actor_ref);

    // Spawn our remote relay actor.
    let options = ActorOptions::default();
    RemoteRelay::<()>::bind::<_, String>(&mut runtime, local_address, router, options)?;

    info!("listening for messages on {}", local_address);

    runtime.start().map_err(rt::Error::map_type)
}

async fn greeter_actor(mut ctx: actor::Context<String, ThreadSafe>) -> Result<(), !> {
    loop {
        let msg = ctx.receive_next().await;
        println!("Hello {}!", msg);
    }
}
