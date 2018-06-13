extern crate actor;
extern crate futures_core;

use actor::system::{ActorSystem, ActorOptions};
use actor::actor::Actor;
use actor::initiator::NoInitiator;
use futures_core::future::{FutureResult, ok};

// Our actor that will greet people and/or things.
struct GreetingActor {
    message: &'static str,
}

// Our Actor implementation.
impl<'a> Actor<'a> for GreetingActor {
    // The type of message we can receive.
    type Message = String;
    // The type of errors we can generate.
    type Error = ();
    // The type of the returned future.
    type Future = FutureResult<(), ()>;

    // The function that will be called once a message is received.
    fn handle(&'a mut self, name: Self::Message) -> Self::Future {
        // Print a greeting message.
        println!("{} {}", self.message, name);
        ok(())
    }
}

fn main() {
    // Create our actor.
    let actor = GreetingActor { message: "Hello" };

    // Create a new actor system, which will run the actors.
    let mut actor_system = ActorSystem::default();

    // Add our actor to the actor system.
    let mut actor_ref = actor_system.add_actor(actor, ActorOptions::default());

    // Send our actor a message.
    actor_ref.send("World".to_owned())
        .expect("unable to send message");

    // Run our actor system. This should cause "Hello World" to be printed.
    actor_system.run::<NoInitiator>(&mut [])
        .expect("unable to run actor system");
}
