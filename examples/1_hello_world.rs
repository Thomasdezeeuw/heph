#![feature(async_await, await_macro, futures_api, never_type)]

use std::borrow::Cow;

use actor::actor::{ActorContext, actor_factory};
use actor::system::{ActorSystemBuilder, ActorOptions};

/// The type of message our actor can receive.
type ActorMessage = Cow<'static, str>;

/// Our greeter actor.
///
/// This function actually implements the `NewActor` trait required by
/// `TcpListener` (see main). This is the reason why we write the strange
/// `(stream, address)` form in the arguments.
async fn greeter_actor(mut ctx: ActorContext<ActorMessage>, message: &'static str) -> Result<(), !> {
    // Try to receive a name and print it.
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

    // Create a new actor factory. This is used to create a new actor on each
    // thread, although we'll only start a single thread.
    let new_actor = actor_factory(greeter_actor);
    // Add our actor to the actor system, along with the starting item, in our
    // case a message for the greeter. We'll use the default options here as
    // well.
    let mut actor_ref = actor_system.add_actor(new_actor, "Hello", ActorOptions::default());

    // Send our actor a message via an `ActorRef`, which is a reference to the
    // actor inside the actor system.
    actor_ref.send("World")
        .expect("unable to send message");

    // Run our actor system. This should cause "Hello World" to be printed and
    // then it should return.
    actor_system.run()
        .expect("unable to run actor system");
}
