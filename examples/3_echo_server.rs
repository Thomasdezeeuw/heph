#![feature(async_await, await_macro, futures_api, never_type)]

use std::io;
use std::net::SocketAddr;

use futures_util::{AsyncReadExt, TryFutureExt};
use log::{info, log};

use heph::actor::{ActorContext, actor_factory};
use heph::log::REQUEST_TARGET;
use heph::net::{TcpListener, TcpStream};
use heph::system::{ActorSystem, ActorOptions, InitiatorOptions};

/// Our actor.
async fn echo_actor(_ctx: ActorContext<!>, (stream, address): (TcpStream, SocketAddr)) -> io::Result<()> {
    // Here we use a special request target to mark this log as a request. This
    // will cause it to be printed to standard out, rather then standard error.
    info!(target: REQUEST_TARGET, "accepted connection: address={}", address);

    let (mut read, mut write) = stream.split();
    await!(read.copy_into(&mut write).map_ok(|_| ()))
}

// The remainder of the example, setting up and running the actor system, is
// the same as example 2.
fn main() {
    let address = "127.0.0.1:7890".parse().unwrap();
    let new_actor = actor_factory(echo_actor);
    let listener = TcpListener::bind(address, new_actor, ActorOptions::default())
        .expect("unable to bind TCP listener");

    ActorSystem::new()
        .with_initiator(listener, InitiatorOptions::default())
        .use_all_cores()
        .enable_logging()
        .run()
        .expect("unable to run actor system");
}
