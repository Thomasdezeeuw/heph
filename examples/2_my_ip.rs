#![feature(futures_api, never_type)]

extern crate actor;
extern crate env_logger;

use std::io::{self, ErrorKind, Write};
use std::net::SocketAddr;
use std::task::Poll;

use actor::actor::{Actor, ActorContext, ActorResult, Status, actor_factory};
use actor::net::{TcpListener, TcpStream};
use actor::system::{ActorSystemBuilder, ActorOptions, InitiatorOptions};

/// Our actor that will print the ip.
struct IpActor {
    /// The TCP connection.
    stream: TcpStream,
    /// The address of the connected connection.
    address: SocketAddr,
}

// Our `Actor` implementation.
impl Actor for IpActor {
    // The type of message we can handle, in our case we don't receive messages.
    type Message = !;
    // The type of errors we can generate. Since we're dealing with I/O, errors
    // are to be expected.
    type Error = io::Error;

    fn handle(&mut self, _: &mut ActorContext, _: Self::Message) -> ActorResult<Self::Error> {
        // This actor doesn't receive messages and thus this is never called.
        unreachable!("EchoActor.poll called");
    }

    // For actors used in an `Initator` this will likely be the starting point.
    fn poll(&mut self, ctx: &mut ActorContext) -> ActorResult<Self::Error> {
        // TODO: use AsyncWrite once available.
        match write!(self.stream, "{}", self.address.ip()) {
            Ok(_) => Poll::Ready(Ok(Status::Complete)),
            Err(ref err) if err.kind() == ErrorKind::WouldBlock => return Poll::Pending,
            Err(ref err) if err.kind() == ErrorKind::Interrupted => self.poll(ctx), // Try again.
            Err(err) => return Poll::Ready(Err(err)),
        }
    }
}

fn main() {
    // Enable logging via the `RUST_LOG` environment variable.
    env_logger::init();

    // Create a new actor factory, that implements the `NewActor` trait.
    let actor_factory = actor_factory(|(stream, address)| IpActor { stream, address } );

    // Create our TCP listener, with an address to listen on, a way to create a
    // new `Actor` for each incoming connection and the options for each actor
    // (for which we'll use the default).
    let address = "127.0.0.1:7891".parse().unwrap();
    let listener = TcpListener::bind(address, actor_factory, ActorOptions::default())
        .expect("unable to bind TCP listener");

    // Create a new actor system, same as in example 1.
    let mut actor_system = ActorSystemBuilder::default().build()
        .expect("unable to build the actor system");

    // Add our initiator.
    actor_system.add_initiator(listener, InitiatorOptions::default())
        .expect("unable to add listener to actor system");

    // And run the system.
    //
    // Because the actor system now has an initiator this will never return,
    // until it receive a stopping signal, e.g. `SIGINT`.
    actor_system.run().expect("unable to run actor system");
}
