#![feature(async_await, await_macro, futures_api, never_type)]

use std::io;
use std::net::SocketAddr;

use futures_util::AsyncWriteExt;
use log::{info, log};

use heph::actor::{ActorContext, actor_factory};
use heph::net::{TcpListener, TcpStream};
use heph::system::{ActorSystem, ActorOptions, InitiatorOptions};

/// Our connection actor. This get called each type we accept an new connection.
///
/// This actor will not receive any message and thus uses `!` (the never type)
/// as message type.
///
/// This function actually implements the `NewActor` trait required by
/// `TcpListener` (see main). This is the reason why we use a tuple
/// `(stream, address)` as a single argument.
async fn conn_actor(_ctx: ActorContext<!>, (mut stream, address): (TcpStream, SocketAddr)) -> io::Result<()> {
    info!("accepted connection: address={}", address);

    // This will allocate a new string which isn't the most efficient way to do
    // this, but it's the easiest so we'll keep this for sake of example.
    let ip = address.ip().to_string();

    // Next we'll write the ip address to the connection.
    await!(stream.write_all(ip.as_bytes()))
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

    // First we create our actor system.
    ActorSystem::new()
        // Next we add our TCP listener.
        .with_initiator(listener, InitiatorOptions::default())
        // We'll create a thread per available cpu core.
        .use_all_cores()
        // And finally we run it.
        .run()
        .expect("unable to run actor system");
}
