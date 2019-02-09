#![feature(async_await, await_macro, futures_api, never_type)]

use heph::actor::Context;
use heph::supervisor::NoSupervisor;
use heph::system::{ActorOptions, ActorSystem, ActorSystemRef, RuntimeError};

fn main() -> Result<(), RuntimeError> {
    // First we create our actor system.
    ActorSystem::new()
        // We add a setup function, which adds our greeter actor.
        .with_setup(add_greeter_actor)
        // And then we run it.
        .run()
}

/// The is the setup function used in the actor system.
fn add_greeter_actor(mut system_ref: ActorSystemRef) -> Result<(), !> {
    // Add our `greeter_actor` to the actor system.
    // All actors need supervision, however our actor doesn't return an error
    // (it uses `!`, the never type, as error), because of this we'll use the
    // `NoSupervisor`, which is a supervisor that does nothing and can't be
    // called.
    // Along with the supervisor we'll also supply the argument to start the
    // actor, in our case this is `()` since our actor doesn't accept any
    // arguments.
    // We'll use the default actor options here, other examples expand on the
    // options available.
    let actor = greeter_actor as fn(_) -> _;
    let mut actor_ref = system_ref.spawn(NoSupervisor, actor, (), ActorOptions::default());

    // By default actors don't do anything when added to the actor system. We
    // need to wake them, for example by sending them a message. If we didn't
    // send this message the system would run forever, without ever making
    // progress (try this by commenting out the send below!).
    // So we'll send our actor a message via an actor reference, which is a
    // reference to the actor inside the actor system. This can be done in two
    // ways, by calling the `send` method or using the `<<=` operator (both do
    // the same thing).
    actor_ref <<= "World";

    Ok(())
}

/// Our greeter actor.
///
/// We'll receive a single message and print it.
async fn greeter_actor(mut ctx: Context<&'static str>) -> Result<(), !> {
    // All actors have an actor context, which give the actor access to its
    // inbox, from which we can `receive` a message.
    let name = await!(ctx.receive_next());
    println!("Hello {}", name);
    Ok(())
}
