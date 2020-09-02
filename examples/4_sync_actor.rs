#![feature(never_type)]

use heph::actor::sync::SyncContext;
use heph::supervisor::NoSupervisor;
use heph::{rt, Runtime};

fn main() -> Result<(), rt::Error> {
    // Spawning synchronous actor works slightly differently the spawning
    // regular (asynchronous) actors. Mainly, synchronous actors need to be
    // spawned before the runtime is started.
    let mut runtime = Runtime::new()?;

    // Spawn a new synchronous actor, returning an actor reference to it.
    let actor = actor as fn(_, _) -> _;
    let mut actor_ref = runtime.spawn_sync_actor(NoSupervisor, actor, "Bye")?;

    // Just like with any actor reference we can send the actor a message.
    actor_ref <<= "Hello world".to_string();
    // We need to drop the reference here to ensure the actor stops.
    // The actor contains a `while` loop receiving messages (see the `actor`
    // function), that only stops iterating once all actor reference to it are
    // dropped or waits otherwise. If didn't manually dropped the reference here
    // it would be dropped only after `runtime.start` returned below, when it
    // goes out of scope. However that will never happen as the `actor` will
    // wait until its dropped.
    drop(actor_ref);

    // And now we start the runtime.
    runtime.start()
}

fn actor(mut ctx: SyncContext<String>, exit_msg: &'static str) -> Result<(), !> {
    while let Ok(msg) = ctx.receive_next() {
        println!("Got a message: {}", msg);
    }
    println!("{}", exit_msg);
    Ok(())
}
