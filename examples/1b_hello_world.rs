#![feature(never_type)]

extern crate actor;
extern crate env_logger;

use actor::actor::{Status, actor_fn};
use actor::system::{ActorSystemBuilder, ActorOptions};

fn main() {
    // Enable logging via the `RUST_LOG` environment variable.
    env_logger::init();

    // In this example we'll use the `ActorFn` helper to create our `Actor`.
    let message = "Hello";
    let actor = actor_fn(move |_, name: String| -> Result<Status, !> {
        println!("{} {}", message, name);
        Ok(Status::Ready)
    });

    // The remainder of the example is the same as example 1.

    let mut actor_system = ActorSystemBuilder::default().build()
        .expect("unable to build the actor system");
    let mut actor_ref = actor_system.add_actor(actor, ActorOptions::default());

    actor_ref.send("World".to_owned())
        .expect("unable to send message");

    actor_system.run()
        .expect("unable to run actor system");
}
