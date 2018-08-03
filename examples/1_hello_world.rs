#![feature(async_await, await_macro, futures_api, never_type)]

use heph::actor::{ActorContext, actor_factory};
use heph::system::{ActorSystemBuilder, ActorOptions};

/// Our greeter actor.
async fn greeter_actor(mut ctx: ActorContext<&'static str>, message: &'static str) -> Result<(), !> {
    let name = await!(ctx.receive());
    println!("{} {}", message, name);
    Ok(())
}

fn main() {
    // Enable logging via the `RUST_LOG` environment variable.
    env_logger::init();

    // Create a new actor system, which will run the actor. We'll just use the
    // default options for the system.
    let mut actor_system = ActorSystemBuilder::default().build()
        .expect("unable to build the actor system");

    // Create a new actor factory. This is used to implement the `NewActor`
    // trait.
    let new_actor = actor_factory(greeter_actor);
    // Add our actor to the actor system, along with the starting item. We'll
    // use the default options here as well.
    let mut actor_ref = actor_system.add_actor(new_actor, "Hello", ActorOptions::default());

    // By default actor don't do anything when adding to the actor system. We
    // need to wake them, for example by sending them a message.
    // Send our actor a message via an `LocalActorRef`, which is a reference to
    // the actor inside the actor system.
    actor_ref.send("World")
        .expect("unable to send message");

    // Run our actor system. This should cause "Hello World" to be printed and
    // then it should return.
    actor_system.run()
        .expect("unable to run actor system");
}
