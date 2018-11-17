#![feature(async_await, await_macro, futures_api, never_type)]

use std::io;

use heph::actor::ActorContext;
use heph::supervisor::NoopSupervisor;
use heph::system::{ActorSystem, ActorSystemRef, ActorOptions};

/// Our greeter actor.
///
/// We'll receive a single message, print it then the actor is done.
async fn greeter_actor(mut ctx: ActorContext<&'static str>) -> Result<(), !> {
    let name = await!(ctx.receive());
    println!("Hello {}", name);
    Ok(())
}

/// The is the setup function used in the actor system.
fn add_greeter_actor(mut system_ref: ActorSystemRef) -> io::Result<()> {
    // Add our `greeter_actor` to the actor system.
    // All actors need supervision, however our actor doesn't return an error
    // (it uses `!` as error type), because of this we'll use the
    // `NoopSupervisor`, which is a no-op supervisor.
    // Along with the supervisor we'll also supply the argument to start the
    // actor, in our case this is `()` since our actor doesn't accept arguments.
    // We'll use the default actor options here.
    let mut actor_ref = system_ref.spawn(NoopSupervisor, greeter_actor as fn(_) -> _, (), ActorOptions::default());

    // By default actors don't do anything when added to the actor system. We
    // need to wake them, for example by sending them a message.
    // So we'll send our actor a message via an `LocalActorRef`, which is a
    // reference to the actor inside the actor system.
    actor_ref.send("World")
        .expect("unable to send message");

    Ok(())
}

fn main() {
    // First we create our actor system with the default options.
    ActorSystem::new()
        // We add a setup function which adds our greeter actor.
        .with_setup(add_greeter_actor)
        // And finally we run it.
        .run()
        .expect("unable to run actor system");
}
