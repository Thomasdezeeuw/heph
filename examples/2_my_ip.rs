#![feature(async_await, await_macro, futures_api, never_type)]

use std::net::SocketAddr;

use futures_util::AsyncWriteExt;
use log::{error, info, log};

use actor::actor::{ActorContext, actor_factory};
use actor::net::{TcpListener, TcpStream};
use actor::system::{ActorSystemBuilder, ActorOptions, InitiatorOptions};

/// Our connection actor.
///
/// This function actually implements the `NewActor` trait required by
/// `TcpListener` (see main). This is the reason why we write the strange
/// `(stream, address)` form in the arguments.
async fn conn_actor(_ctx: ActorContext<!>, (mut stream, address): (TcpStream, SocketAddr)) -> Result<(), !> {
    info!("accepted connection: address={}", address);

    // This will allocate a new string which isn't the most efficient way to do
    // this, but it's the easiest so we'll keep this for sake of example.
    let ip = address.ip().to_string();

    match await!(stream.write_all(ip.as_bytes())) {
        Ok(()) => Ok(()),
        Err(err) => {
            // TODO: maybe we should return the error to the supervisor, let it
            // log and say there we can't do much.
            error!("error writing ip: {}, address={}", err, address);

            // This seems odd, even though we've encountered an error we're
            // returned Ok. We do this because any error returned would be send
            // to the supervisor, but it won't be able to improve the situation
            // either. All we can do is drop the connection and let the user try
            // again.
            Ok(())
        },
    }
}

fn main() {
    // Enable logging via the `RUST_LOG` environment variable.
    env_logger::init();

    // Create our TCP listener, with an address to listen, a way to create a new
    // actor for each incoming connections and the options for each actor (for
    // which we'll use the default).
    let address = "127.0.0.1:7890".parse().unwrap();
    let new_actor = actor_factory(conn_actor);
    let listener = TcpListener::bind(address, new_actor, ActorOptions::default())
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
    // until it receives a stopping signal, e.g. `SIGINT` (press CTRL+C).
    actor_system.run()
        .expect("unable to run actor system");
}
