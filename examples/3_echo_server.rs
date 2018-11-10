#![feature(async_await, await_macro, futures_api, never_type)]

use std::io;
use std::net::SocketAddr;

use futures_util::{AsyncReadExt, TryFutureExt};
use log::{error, info};

use heph::actor::{ActorContext, actor_factory};
use heph::log::REQUEST_TARGET;
use heph::net::{TcpListener, TcpStream};
use heph::supervisor::SupervisorStrategy;
use heph::system::{ActorSystem, ActorOptions, InitiatorOptions};

/// Our actor.
async fn echo_actor(_ctx: ActorContext<!>, (stream, address): (TcpStream, SocketAddr)) -> io::Result<()> {
    // Here we use a special request target to mark this log as a request. This
    // will cause it to be printed to standard out, rather then standard error.
    info!(target: REQUEST_TARGET, "accepted connection: address={}", address);

    let (mut read, mut write) = stream.split();
    await!(read.copy_into(&mut write).map_ok(|_| ()))
}

// The remainder of the example, the supervisor, setting up and running the
// actor system, is the same as example 2.

fn echo_supervisor(err: io::Error) -> SupervisorStrategy<(TcpStream, SocketAddr)> {
    error!("error handling connection: {}", err);
    SupervisorStrategy::Stop
}

fn main() {
    heph::log::init();

    let address = "127.0.0.1:7890".parse().unwrap();
    let new_actor = actor_factory(echo_actor);
    let listener = TcpListener::bind(address, echo_supervisor, new_actor, ActorOptions::default())
        .expect("unable to bind TCP listener");
    info!("listening: address={}", address);

    ActorSystem::new()
        .with_initiator(listener, InitiatorOptions::default())
        .use_all_cores()
        .run()
        .expect("unable to run actor system");
}
