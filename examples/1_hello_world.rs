#![feature(async_await, await_macro, futures_api, never_type)]

use std::io;

use heph::actor::{ActorContext, actor_factory};
use heph::system::{ActorSystem, ActorSystemRef, ActorOptions};

/// Our greeter actor.
///
/// We'll receive a single message, print it then the actor is done.
async fn greeter_actor(mut ctx: ActorContext<&'static str>, message: &'static str) -> Result<(), !> {
    let name = await!(ctx.receive());
    println!("{} {}", message, name);
    Ok(())
}

/// The is the setup function used in the actor system.
fn add_greeter_actor(mut system_ref: ActorSystemRef) -> io::Result<()> {
    // Create a new actor factory. This is used to implement the `NewActor`
    // trait.
    let new_actor = actor_factory(greeter_actor);

    // Add our actor to the actor system, along with the starting item. We'll
    // use the default actor options here.
    let mut actor_ref = system_ref.add_actor(new_actor, "Hello", ActorOptions::default());

    // By default actor don't do anything when added to the actor system. We
    // need to wake them, for example by sending them a message.
    // Send our actor a message via an `LocalActorRef`, which is a reference to
    // the actor inside the actor system.
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
