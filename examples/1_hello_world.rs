// Copyright 2017 Thomas de Zeeuw
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT
// or http://opensource.org/licenses/MIT>, at your option. This file may not be
// used, copied, modified, or distributed except according to those terms.

//! A hello world example, run with `cargo run --example 1_hello_world`.

extern crate actor;
#[macro_use]
extern crate log;
extern crate std_logger;

// We'll use the prelude which imports all useful types of the actor system.
use actor::prelude::*;

fn main() {
    // Initialized our logging implementation, any logging implementation will
    // do. See the `log` crate, or crates.io, for more logging implementations.
    // In this logging implementation the enviroment variable `LOG_LEVEL` can be
    // used to change the logging detail, try setting it to "TRACE" for example.
    std_logger::init();

    // Create a new actor system, with the default settings.
    let mut system = ActorSystem::default();

    // Add our printing actor.
    let mut print_actor_ref = system.add_actor(PrintActor);

    // Now we have an `ActorRef`, which we can use to send messages to the
    // actor. So we'll send our "Hello world" message.
    // Note that nothing will happen yet after this step.
    print_actor_ref.send("Hello world".to_owned());

    // Now run the actor system. This will block until all actors have completed
    // there jobs, in our case printing "Hello world".
    system.run()
        .unwrap_or_else(|err| error!("error running actor system: {}", err));
}

/// Our actor that will print any messages it receives.
struct PrintActor;

impl Actor for PrintActor {
    /// The type of message we can receive.
    type Message = String;
    /// The type of errors this actor can return to it's supervisor. This is not
    /// used in this example, since we don't have any errors to report.
    type Error = ();
    /// This is the "Future" is returned by the handle method, it's actually the
    /// `IntoFuture` trait so `Result`s can be returned as well.
    type Future = Result<(), Self::Error>;

    /// This is the method that gets called once a new message is available.
    fn handle(&mut self, msg: Self::Message) -> Self::Future {
        // We print and then we're done.
        println!("{}", msg);
        Ok(())
    }
}
