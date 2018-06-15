extern crate actor;
extern crate futures_core;

use actor::system::{ActorSystemBuilder, ActorOptions};
use actor::actor::Actor;
use actor::initiator::NoInitiator;
use futures_core::{Future, Async, Poll};
use futures_core::task::Context;

// Our actor that will greet people and/or things.
#[derive(Debug)]
struct GreetingActor {
    message: &'static str,
}

impl Future for GreetingActor {
    // The returned value, must always be empty.
    type Item = ();
    // The type of errors we can generate, in our cause none.
    type Error = ();

    // Since our `handle` implementation always returns `Async::Ready`, this
    // will never be called.
    fn poll(&mut self, _: &mut Context) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }
}

// Our Actor implementation.
impl Actor for GreetingActor {
    // The type of message we can receive.
    type Message = String;

    // The function that will be called once a message is received.
    fn handle(&mut self, _: &mut Context, name: Self::Message) -> Poll<(), Self::Error> {
        // Print a greeting message.
        println!("{} {}", self.message, name);
        Ok(Async::Ready(()))
    }
}

fn main() {
    // Create our actor.
    let actor = GreetingActor { message: "Hello" };

    // Create a new actor system, which will run the actors.
    let mut actor_system = ActorSystemBuilder::default().build()
        .expect("unable to build `ActorSystem`");

    // Add our actor to the actor system.
    let mut actor_ref = actor_system.add_actor(actor, ActorOptions::default())
        .expect("unable to add actor to `ActorSystem`");

    // Send our actor a message.
    actor_ref.send("World".to_owned())
        .expect("unable to send message");

    // Run our actor system. This should cause "Hello World" to be printed.
    actor_system.run::<NoInitiator>(&mut [])
        .expect("unable to run actor system");
}
