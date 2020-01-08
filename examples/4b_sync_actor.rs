#![feature(never_type)]

use std::io;

use heph::actor::sync::SyncContext;
use heph::supervisor::{NoSupervisor, SupervisorStrategy};
use heph::{actor, ActorOptions, Runtime, RuntimeError, RuntimeRef};

fn main() -> Result<(), RuntimeError> {
    Runtime::new().with_setup(setup).start()
}

fn setup(mut runtime_ref: RuntimeRef) -> Result<(), !> {
    let actor = sync_actor_spawned as fn(_) -> _;
    let options = ActorOptions::default().schedule();
    runtime_ref.spawn(spawner_supervisor, actor, (), options);
    Ok(())
}

fn spawner_supervisor(_: io::Error) -> SupervisorStrategy<()> {
    SupervisorStrategy::Stop
}

/// Our asynchronous actor that spawn a synchronous actor.
async fn sync_actor_spawned(mut ctx: actor::Context<&'static str>) -> io::Result<()> {
    let actor = actor as fn(_, _) -> _;
    let mut actor_ref = ctx.runtime().clone()
        .spawn_sync_actor(&mut ctx, NoSupervisor, actor, "Bye")?.await;
    actor_ref <<= "Hello world".to_string();
    Ok(())
}

/// Our synchronous actor, same as example 4.
fn actor(mut ctx: SyncContext<String>, exit_msg: &'static str) -> Result<(), !> {
    if let Ok(msg) = ctx.receive_next() {
        println!("Got a message: {}", msg);
    } else {
        eprintln!("Receive no messages");
    }
    println!("{}", exit_msg);
    Ok(())
}
