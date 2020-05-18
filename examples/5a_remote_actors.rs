#![feature(never_type)]

use heph::actor::context::ThreadSafe;
use heph::net::rpc::RemoteRegistry;
use heph::supervisor::NoSupervisor;
use heph::{actor, rt, ActorOptions, Runtime};

fn main() -> Result<(), rt::Error> {
    heph::log::init();

    let mut runtime = Runtime::new()?;

    // Start our greeter actor.
    #[allow(trivial_casts)]
    let greeter_actor = greeter_actor as fn(_) -> _;
    let options = ActorOptions::default().mark_ready();
    let actor_ref = runtime.spawn(NoSupervisor, greeter_actor, (), options);

    // Create a new remote registry.
    let mut registry = RemoteRegistry::new();
    // Register our actor so it can receive messages from actor on remote nodes.
    registry
        .register("greeter", actor_ref)
        .expect("failed to register actor");
    // Spawn the listener that relays the messages for us.
    let address = "127.0.0.1:9001".parse().unwrap();
    let _actor_ref = registry.spawn_listener(&mut runtime, address);
    // Let remote listener receive process signals.
    // TODO: add below.
    //runtime.receive_signals(actor_ref);

    runtime.start()
}

async fn greeter_actor(mut ctx: actor::Context<String, ThreadSafe>) -> Result<(), !> {
    // TODO: change to:
    // while let Some(msg) = ctx.receive_next().await {
    loop {
        let msg = ctx.receive_next().await;
        println!("Hello {}!", msg);
    }
    //Ok(())
}
