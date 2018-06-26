#![feature(futures_api, never_type)]

extern crate actor;
extern crate env_logger;

use std::io::{self, ErrorKind, Read, Write};
use std::task::Poll;

use actor::actor::{Actor, ActorContext, ActorResult, Status, actor_factory};
use actor::net::{TcpListener, TcpStream};
use actor::system::{ActorSystemBuilder, ActorOptions, InitiatorOptions};

/// Our actor that will echo back onto the TCP stream.
struct EchoActor {
    stream: TcpStream,
    buffer: Vec<u8>,
}

// TODO: remove this any use `AsyncRead` and `AsyncWrite` once available.
// Little helper macro for the I/O operations.
macro_rules! try_io {
    ($op:expr) => (
        match $op {
            Ok(ok) => ok,
            Err(ref err) if err.kind() == ErrorKind::WouldBlock => return Poll::Pending,
            // TODO: try again:
            //Err(ref err) if err.kind() == ErrorKind::Interrupted => continue, // Try again.
            Err(err) => return Poll::Ready(Err(err)),
        };
    );
}

// Our `Actor` implementation.
impl Actor for EchoActor {
    // The type of message we can handle, in our case we don't receive messages.
    type Message = !;
    // The type of errors we can generate. Since we're dealing with I/O, errors
    // are to be expected.
    type Error = io::Error;

    fn handle(&mut self, _: &mut ActorContext, _: Self::Message) -> ActorResult<Self::Error> {
        // This actor doesn't receive message and thus this is never called.
        unreachable!("EchoActor.poll called");
    }

    // For actors used in an `Initator` this will likely be the starting point.
    fn poll(&mut self, ctx: &mut ActorContext) -> ActorResult<Self::Error> {
        if self.buffer.is_empty() {
            // Try to read from stream.
            match try_io!(self.stream.read(&mut self.buffer)) {
                0 => return Poll::Ready(Ok(Status::Complete)),
                n => {
                    // Set the length of the buffer and go into the other
                    // branch.
                    self.buffer.truncate(n);
                    self.poll(ctx)
                },
            }
        } else {
            // Try to write to stream.
            match try_io!(self.stream.write(&self.buffer)) {
                n if n == self.buffer.len() => {
                    // Written the entire buffer, try reading again.
                    self.buffer.truncate(0);
                    self.poll(ctx)
                },
                n => {
                    // Haven't written the entire buffer, we'll need to try
                    // again later.
                    self.buffer = self.buffer.split_off(n);
                    Poll::Ready(Ok(Status::Ready))
                },
            }
        }
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
