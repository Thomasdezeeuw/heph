#![feature(futures_api, never_type)]

use std::task::Poll;

use actor::actor::{Actor, ActorContext, ActorResult, Status};
use actor::system::{ActorSystemBuilder, ActorOptions};

// Our actor that will greet people and/or things.
#[derive(Debug)]
struct GreetingActor {
    message: &'static str,
}

// Our `Actor` implementation.
impl Actor for GreetingActor {
    // The type of message we can handle.
    type Message = String;
    // We never return an error.
    type Error = !;

    // The function that will be called once a message is received for the actor.
    fn handle(&mut self, _: &mut ActorContext, name: Self::Message) -> ActorResult<Self::Error> {
        // Print a greeting message.
        println!("{} {}", self.message, name);
        // And that is all we need to do, so we're done.
        Poll::Ready(Ok(Status::Ready))
    }

    fn poll(&mut self, _: &mut ActorContext) -> ActorResult<Self::Error> {
        Poll::Ready(Ok(Status::Ready))
    }
}

// Now our actor is ready lets put it to work.
fn main() {
    // Enable logging via the `RUST_LOG` environment variable.
    env_logger::init();

    // Create our actor.
    let actor = GreetingActor { message: "Hello" };

    // Create a new actor system, which will run the actor. We'll just use the
    // default options for the system.
    let mut actor_system = ActorSystemBuilder::default().build()
        .expect("unable to build the actor system");

    // Add our actor to the actor system. For now we'll use the default options
    // here as well.
    let mut actor_ref = actor_system.add_actor(actor, ActorOptions::default());

    // Send our actor a message via an `ActorRef`, which is a reference to the
    // actor inside the actor system.
    actor_ref.send("World".to_owned())
        .expect("unable to send message");

    // Run our actor system. This should cause "Hello World" to be printed and
    // then it should return.
    actor_system.run()
        .expect("unable to run actor system");
}
