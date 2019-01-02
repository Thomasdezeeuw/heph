#![feature(async_await, await_macro, futures_api, never_type)]

use std::io;

use heph::actor::ActorContext;
use heph::supervisor::NoopSupervisor;
use heph::system::{ActorSystem, ActorSystemRef, ActorOptions, RuntimeError};

/// Our greeter actor.
async fn greeter_actor(_ctx: ActorContext<&'static str>) -> Result<(), !> {
    println!("Hello World");
    Ok(())
}

/// The is the setup function used in the actor system.
fn add_greeter_actor(mut system_ref: ActorSystemRef) -> io::Result<()> {
    // Same as in example 1 we add our actor to the actor system. This time
    // however we use the `schedule` option to automatically schedule our actor
    // to run when added to the actor system. This is needed because otherwise
    // is never run, since it doesn't receive any external events to schedule
    // the actor such as sending it a message in example 1.
    system_ref.spawn(NoopSupervisor, greeter_actor as fn(_) -> _, (),
        ActorOptions {
            schedule: true,
            .. ActorOptions::default()
        });

    Ok(())
}

fn main() -> Result<(), RuntimeError> {
    // The creation and running of the actor system is the same as in example 1.
    ActorSystem::new()
        .with_setup(add_greeter_actor)
        .run()
}
