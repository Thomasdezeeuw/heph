#![feature(never_type)]

use heph::actor::sync::SyncContext;
use heph::supervisor::NoSupervisor;
use heph::{Runtime, RuntimeError};

fn main() -> Result<(), RuntimeError> {
    // Spawning synchronous actor works slightly differently the spawning
    // regular (asynchronous) actors. Mainly, synchronous actors need to be
    // spawned before the runtime is started.
    let mut runtime = Runtime::new();

    // Spawn a new synchronous actor, returning an actor reference to it.
    let actor = actor as fn(_, _) -> _;
    let mut actor_ref = runtime.spawn_sync_actor(NoSupervisor, actor, "Bye")?;

    // Just like with any actor reference we can send the actor a message.
    actor_ref <<= "Hello world".to_string();

    // And now we start the runtime.
    runtime.run()
}

fn actor(mut ctx: SyncContext<String>, exit_msg: &'static str) -> Result<(), !> {
    if let Ok(msg) = ctx.receive_next() {
        println!("Got a message: {}", msg);
    } else {
        eprintln!("Receive no messages");
    }
    println!("{}", exit_msg);
    Ok(())
}
