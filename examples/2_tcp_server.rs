fn main() {}
/*
#![feature(never_type)]

extern crate actor;
extern crate env_logger;
extern crate futures_core;
extern crate futures_io;

use std::io;

use actor::actor::{Actor, actor_factory};
use actor::net::{TcpListener, TcpStream};
use actor::system::{ActorSystemBuilder, ActorOptions, InitiatorOptions};
use futures_core::{Future, Async, Poll};
use futures_core::task::Context;
use futures_io::{AsyncRead, AsyncWrite};

/// Our actor that will echo back onto the TCP stream.
struct EchoActor {
    stream: TcpStream,
    buffer: Vec<u8>,
}

// To implement the `Actor` trait we need to implement the `Future` trait as
// well.
impl Future for EchoActor {
    // The returned value, must always be empty.
    type Item = ();
    // The type of errors we can generate. Since we're dealing with I/O, errors
    // are to be expected.
    type Error = io::Error;

    // For actors used in an `Initator` this will likely be the starting point.
    fn poll(&mut self, ctx: &mut Context) -> Poll<(), Self::Error> {
        if self.buffer.is_empty() {
            // Try to read from stream.
            let res = self.stream.poll_read(ctx, &mut self.buffer);
            if let Ok(Async::Ready(n)) = res {
                self.buffer.truncate(n);
                // If we read something we can call this function again to echo
                // it back.
                self.poll(ctx)
            } else {
                // Otherwise just return the result, mapping the returned size
                // in `()`.
                res.map(|a| a.map(|_| ()))
            }
        } else {
            // Try to write the buffer contents back to the stream.
            let res = self.stream.poll_write(ctx, &self.buffer);
            if let Ok(Async::Ready(n)) = res {
                if n == self.buffer.len() {
                    // Written the entire buffer, so now we can try reading
                    // again.
                    self.buffer.truncate(0);
                    self.poll(ctx)
                } else {
                    // Haven't written the entire buffer, we'll need to try
                    // again later.
                    self.buffer = self.buffer.split_off(n);
                    Ok(Async::Pending)
                }
            } else {
                // Otherwise just return the result, mapping the returned size
                // in `()`.
                res.map(|a| a.map(|_| ()))
            }
        }
    }
}

// Our `Actor` implementation.
impl Actor for EchoActor {
    // The type of message we can handle, in our case we don't receive messages.
    type Message = !;

    // The function that will be called once a message is received for the actor.
    fn handle(&mut self, _: &mut Context, _: Self::Message) -> Poll<(), Self::Error> {
        // This should not be impossible.
        unreachable!("EchoActor.poll called");
    }
}

// Now our actor is ready lets put it to work.
fn main() {
    // Enable logging via the `RUST_LOG` environment variable.
    env_logger::init();

    // Create a new actor factory, that implements the `NewActor` trait.
    let actor_factory = actor_factory(|(stream, _adress)| EchoActor { stream, buffer: vec![0; 1024] });

    // Create our TCP listener, with an address to listen on, a way to create a
    // new `Actor` for each incoming connection and the options for each actor
    // (for which we'll use the default).
    let address = "127.0.0.1:7890".parse().unwrap();
    let listener = TcpListener::bind(address, actor_factory, ActorOptions::default())
        .expect("unable to bind TCP listener");

    // Create a new actor system, which will run the actor. We'll just use the
    // default options for the system.
    let mut actor_system = ActorSystemBuilder::default().build()
        .expect("unable to build the actor system");

    actor_system.add_initiator(listener, InitiatorOptions::default())
        .expect("unable to add listener to actor system");

    // Run our actor system with out TCP listener as initiator.
    //
    // This will block for ever, or until the processes is stopped.
    actor_system.run().expect("unable to run actor system");
}
*/
