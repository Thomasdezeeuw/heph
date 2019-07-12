#![feature(async_await, never_type)]

use std::io;
use std::net::SocketAddr;

use futures_util::AsyncWriteExt;

use heph::log::{self, error, info};
use heph::net::tcp::{self, TcpStream};
use heph::supervisor::SupervisorStrategy;
use heph::system::options::Priority;
use heph::system::RuntimeError;
use heph::{actor, ActorOptions, ActorSystem, ActorSystemRef};

fn main() -> Result<(), RuntimeError<io::Error>> {
    // For this example we'll enable logging, this give us a bit more insight
    // into the running system. By default it only logs informational or more
    // severe messages, the environment variable `LOG_LEVEL` can be set to
    // change this. For example enabling logging of trace severity message can
    // be done by setting `LOG_LEVEL=trace`.
    log::init();

    // Just like in examples 1 and 2 we'll create our actor system and add our
    // setup function. But for this example we'll create a worker thread per
    // available CPU core.
    ActorSystem::new().with_setup(setup).use_all_cores().run()
}

/// Our setup function that will add the TCP listener to the actor system.
fn setup(mut system_ref: ActorSystemRef) -> io::Result<()> {
    // Create our TCP listener. This TCP listener will create a new actor for
    // each incoming TCP stream. As always, actors needs supervision, this is
    // done by `conn_supervisor` in this example. And as each actor will need to
    // be added to the actor system it needs the `ActorOptions` to do that,
    // we'll use the defaults options here.
    let actor = conn_actor as fn(_, _, _) -> _;
    let server = tcp::setup_server(conn_supervisor, actor, ActorOptions::default());

    // As the TCP listener is just another actor we need to spawn it like any
    // other actor. And again actors needs supervision, thus we provide
    // `listener_supervisor` as supervisor. As argument the TCP listener needs
    // an address to listen on.
    let address = "127.0.0.1:7890".parse().unwrap();
    // We'll give our listener a low priority to prioritise handling of ongoing
    // requests over accepting new requests possibly overloading the system.
    let options = ActorOptions::default().with_priority(Priority::LOW);
    system_ref.try_spawn(server_supervisor, server, address, options)?;

    Ok(())
}

/// Our supervisor for the TCP listener.
///
/// In this example we'll log the error and then stop the actor, but we could
/// restart it by providing another address.
fn server_supervisor(err: tcp::ServerError<!>) -> SupervisorStrategy<(SocketAddr)> {
    error!("error in TCP listener: {}", err);
    SupervisorStrategy::Stop
}

/// Our supervisor for the connection actor.
///
/// Since we can't create a new TCP connection all this supervisor does is log
/// the error and signal to stop the actor.
fn conn_supervisor(err: io::Error) -> SupervisorStrategy<(TcpStream, SocketAddr)> {
    error!("error handling connection: {}", err);
    SupervisorStrategy::Stop
}

/// Our connection actor.
///
/// This actor will not receive any message and thus uses `!` (the never type)
/// as message type.
async fn conn_actor(
    _: actor::Context<!>,
    mut stream: TcpStream,
    address: SocketAddr,
) -> io::Result<()> {
    info!("accepted connection: address={}", address);

    // This will allocate a new string which isn't the most efficient way to do
    // this, but it's the easiest so we'll keep this for sake of example.
    let ip = address.ip().to_string();

    // Next we'll write the IP address to the connection.
    stream.write_all(ip.as_bytes()).await
}
