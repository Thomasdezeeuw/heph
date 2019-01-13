#![feature(async_await, await_macro, futures_api, never_type)]

use std::io;
use std::net::SocketAddr;

use futures_util::AsyncWriteExt;
use log::{error, info};

use heph::actor::ActorContext;
use heph::net::{TcpListener, TcpStream};
use heph::supervisor::SupervisorStrategy;
use heph::system::options::Priority;
use heph::system::{ActorSystem, ActorSystemRef, ActorOptions, RuntimeError};

fn main() -> Result<(), RuntimeError> {
    // Enable logging.
    heph::log::init();

    // First we create our actor system.
    ActorSystem::new()
        // Next we add our setup function that will add the TCP listener.
        .with_setup(setup)
        // We'll create a worker thread per available cpu core.
        //.use_all_cores()
        .num_threads(1)
        // And finally we run it.
        .run()
}

/// Our setup function that will add the TCP listener to the actor system.
fn setup(mut system_ref: ActorSystemRef) -> io::Result<()> {
    // Create our TCP listener. This TCP listener will create a new actor, in
    // our case `conn_actor`, for each incoming TCP stream. This will actor will
    // need supervision which is done by `conn_supervisor` in this example. And
    // as each actor will need to be added to the actor system it needs the
    // `ActorOptions` to add it, we'll use the defaults options here.
    let listener = TcpListener::new(conn_supervisor, conn_actor as fn(_, _, _) -> _, ActorOptions::default());

    // As a TCP listener is just another actor we need to spawn it like any
    // other actor, for which we'll need an address as argument. And as any
    // other actor it needs supervision, thus we provide `listener_supervisor`
    // as supervisor.
    let address = "127.0.0.1:7890".parse().unwrap();
    system_ref.spawn(listener_supervisor, listener, address, ActorOptions {
        // We'll give our listener a low priority to prioritise handling of
        // ongoing requests over accepting new requests possibly overloading the
        // system.
        priority: Priority::LOW,
        .. Default::default()
    })?;

    Ok(())
}

/// Our supervisor for the TCP listener.
///
/// In this example we'll log the error and then stop the actor, but we could
/// restart it by providing another address.
fn listener_supervisor(err: io::Error) -> SupervisorStrategy<(SocketAddr)> {
    error!("error accepting connection: {}", err);
    SupervisorStrategy::Stop
}

/// Our connection actor. This get called each type we accept an new connection.
///
/// This actor will not receive any message and thus uses `!` (the never type)
/// as message type.
///
/// This function actually implements the `NewActor` trait required by
/// `TcpListener` (see main). This is the reason why we use a tuple
/// `(stream, address)` as a single argument.
async fn conn_actor(_ctx: ActorContext<!>, mut stream: TcpStream, address: SocketAddr) -> io::Result<()> {
    info!("accepted connection: address={}", address);

    // This will allocate a new string which isn't the most efficient way to do
    // this, but it's the easiest so we'll keep this for sake of example.
    let ip = address.ip().to_string();

    // Next we'll write the ip address to the connection.
    await!(stream.write_all(ip.as_bytes()))
}

/// Our connection actor supervisor.
///
/// Since we can't create a new TCP connection all this supervisor does is log
/// the error and signal to stop the actor.
fn conn_supervisor(err: io::Error) -> SupervisorStrategy<(TcpStream, SocketAddr)> {
    error!("error handling connection: {}", err);
    SupervisorStrategy::Stop
}
