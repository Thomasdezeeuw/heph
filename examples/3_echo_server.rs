#![feature(async_await, await_macro, futures_api, never_type)]

use std::net::SocketAddr;

use futures_util::AsyncReadExt;
use log::{error, info, log};

use heph::actor::{ActorContext, actor_factory};
use heph::net::{TcpListener, TcpStream};
use heph::system::{ActorSystemBuilder, ActorOptions, InitiatorOptions};

/// Our actor.
async fn echo_actor(_ctx: ActorContext<!>, (stream, address): (TcpStream, SocketAddr)) -> Result<(), !> {
    info!("accepted connection: address={}", address);

    let (mut read, mut write) = stream.split();
    match await!(read.copy_into(&mut write)) {
        Ok(_) => Ok(()),
        Err(err) => {
            // TODO: maybe we should return the error to the supervisor, let it
            // log and say there we can't do much.
            error!("error echoing stream: {}, address={}", err, address);

            // This seems odd, even though we've encountered an error we're
            // returned Ok. We do this because any error returned would be send
            // to the supervisor, but it won't be able to improve the situation
            // either. All we can do is drop the connection and let the user try
            // again.
            Ok(())
        },
    }
}

// The remainder of the example, setting up and running the actor system, is
// the same as example 2.
fn main() {
    env_logger::init();

    let address = "127.0.0.1:7890".parse().unwrap();
    let new_actor = actor_factory(echo_actor);
    let listener = TcpListener::bind(address, new_actor, ActorOptions::default())
        .expect("unable to bind TCP listener");

    let mut actor_system = ActorSystemBuilder::default().build()
        .expect("unable to build the actor system");

    actor_system.add_initiator(listener, InitiatorOptions::default())
        .expect("unable to add listener to actor system");

    actor_system.run()
        .expect("unable to run actor system");
}
