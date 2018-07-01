#![feature(futures_api, never_type)]

extern crate actor;
extern crate env_logger;
#[macro_use]
extern crate log;

use std::io::{self, ErrorKind, Read, Write};
use std::task::Poll;
use std::net::SocketAddr;

use actor::actor::{Actor, NewActor, ActorContext, ActorResult, Status};
use actor::net::{TcpListener, TcpStream};
use actor::system::{ActorSystemBuilder, ActorOptions, InitiatorOptions};

/// Our actor that will echo back onto the TCP stream.
#[derive(Debug)]
struct EchoActor {
    /// The TCP connection.
    stream: TcpStream,
    /// Buffer to hold the read contents.
    buffer: Vec<u8>,
}

impl Actor for EchoActor {
    // The type of message we can handle, in our case we don't receive messages.
    type Message = !;
    // The type of errors we can generate. Since we're dealing with I/O, errors
    // are to be expected.
    type Error = io::Error;

    fn handle(&mut self, _: &mut ActorContext, _: Self::Message) -> ActorResult<Self::Error> {
        // This actor doesn't receive messages and thus this is never called.
        unreachable!("EchoActor.poll called");
    }

    // For actors used in an `Initiator` this will likely be the starting point.
    fn poll(&mut self, ctx: &mut ActorContext) -> ActorResult<Self::Error> {
        // TODO: use AsyncRead and AsyncWrite once available.

        if self.buffer.is_empty() {
            // Try to read from stream.
            // NOTE: do NOT use this in actual production code, this is a DOS
            // attack waiting to happen. We only use this for simplicity sake.
            match self.stream.read_to_end(&mut self.buffer) {
                // Read everything from the stream, so we're done.
                Ok(0) => Poll::Ready(Ok(Status::Complete)),
                // Move to writing part.
                Ok(_) => self.poll(ctx),
                Err(ref err) if err.kind() == ErrorKind::WouldBlock => {
                    if self.buffer.is_empty() {
                        // Nothing it read, so we'll have to wait.
                        Poll::Pending
                    } else {
                        // Echo back what we have now.
                        self.poll(ctx)
                    }
                },
                // NOTE: `ErrorKind::Interrupted` is handled for us by
                // `read_to_end`, otherwise we should handle it here.
                Err(err) => Poll::Ready(Err(err)),
            }
        } else {
            // Try to echo back the buffer to the stream.
            match self.stream.write(&self.buffer) {
                Ok(n) if n == self.buffer.len() => {
                    // Written the entire buffer, so try reading again.
                    self.buffer.truncate(0);
                    self.poll(ctx)
                },
                Ok(n) => {
                    // Not the entire buffer is written, we need to try again
                    // later.
                    self.buffer.drain(0..n);
                    Poll::Pending
                },
                Err(ref err) if err.kind() == ErrorKind::WouldBlock => Poll::Pending,
                Err(err) => Poll::Ready(Err(err)),
            }
        }
    }
}

/// In example 2 we used the `actor_factory` function to implement `NewActor`,
/// here we do it manually.
#[derive(Debug)]
struct NewEchoActor {
    buffer_size: usize,
}

impl NewActor for NewEchoActor {
    type Actor = EchoActor;
    type Item = (TcpStream, SocketAddr);

    fn new(&mut self, (stream, address): Self::Item) -> Self::Actor {
        info!("Accepted connection from: {}", address);
        EchoActor { stream, buffer: Vec::with_capacity(self.buffer_size) }
    }
}

fn main() {
    // Enable logging via the `RUST_LOG` environment variable.
    env_logger::init();

    let actor_factory = NewEchoActor { buffer_size: 128 };

    // The remainder of the example, setting up and running the actor system, is
    // the same as example 2.

    let address = "127.0.0.1:7890".parse().unwrap();
    let listener = TcpListener::bind(address, actor_factory, ActorOptions::default())
        .expect("unable to bind TCP listener");

    let mut actor_system = ActorSystemBuilder::default().build()
        .expect("unable to build the actor system");

    actor_system.add_initiator(listener, InitiatorOptions::default())
        .expect("unable to add listener to actor system");

    actor_system.run().expect("unable to run actor system");
}
