extern crate actor;
extern crate futures_core;

use actor::actor::Actor;
use actor::initiator::NoInitiator;
use actor::system::{ActorSystemBuilder, ActorOptions};
use futures_core::task::Context;
use futures_core::{Future, Async, Poll};

// Our actor that will greet people and/or things.
#[derive(Debug)]
struct GreetingActor {
    message: &'static str,
}

// To implement the `Actor` trait we need to implement the `Future` trait as
// well.
impl Future for GreetingActor {
    // The returned value, must always be empty.
    type Item = ();
    // The type of errors we can generate, in our cause none.
    type Error = ();

    // Since our `handle` method (see Actor implementation below) always returns
    // `Async::Ready`, this will never be called.
    fn poll(&mut self, _: &mut Context) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }
}

// Our `Actor` implementation.
impl Actor for GreetingActor {
    // The type of message we can handle.
    type Message = String;

    // The function that will be called once a message is received for the actor.
    fn handle(&mut self, _: &mut Context, name: Self::Message) -> Poll<(), Self::Error> {
        // Print a greeting message.
        println!("{} {}", self.message, name);
        // And that is all we need to done, so we're done.
        Ok(Async::Ready(()))
    }
}

// Now our actor is ready lets put it to work.
fn main() {
    // Create our actor.
    let actor = GreetingActor { message: "Hello" };

    // Create a new actor system, which will run the actor. We'll just use the
    // default options for the system.
    let mut actor_system = ActorSystemBuilder::default().build()
        .expect("unable to build the actor system");

    // Add our actor to the actor system. For now we'll use the default options
    // here as well.
    let mut actor_ref = actor_system.add_actor(actor, ActorOptions::default())
        .expect("unable to add actor to actor system");

    // Send our actor a message via an `ActorRef`, which is a reference to the
    // actor inside the actor system.
    actor_ref.send("World".to_owned())
        .expect("unable to send message");

    // Run our actor system. This should cause "Hello World" to be printed and
    // then it should return.
    actor_system.run::<NoInitiator>(&mut [])
        .expect("unable to run actor system");
}
