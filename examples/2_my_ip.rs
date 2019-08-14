#![feature(async_await, never_type)]

use std::io;
use std::net::SocketAddr;

use futures_util::AsyncWriteExt;

use heph::log::{self, error, info};
use heph::net::tcp::{self, TcpStream};
use heph::supervisor::{Supervisor, SupervisorStrategy};
use heph::system::options::Priority;
use heph::system::RuntimeError;
use heph::{actor, ActorOptions, ActorSystem, ActorSystemRef, NewActor};

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
    // Create our TCP server. This TCP server will create a new actor for each
    // incoming TCP stream. As always, actors needs supervision, this is done by
    // `ConnSupervisor` in this example. And as each actor will need to be added
    // to the actor system it needs the `ActorOptions` to do that, we'll use the
    // defaults options here.
    let actor = conn_actor as fn(_, _, _) -> _;
    let server = tcp::Server::setup(conn_supervisor, actor, ActorOptions::default());

    // As the TCP server is just another actor we need to spawn it like any
    // other actor. And again actors needs supervision, thus we provide
    // `ServerSupervisor` as supervisor. As argument the TCP listener needs an
    // address to listen on.
    let address = "127.0.0.1:7890".parse().unwrap();
    // We'll give our listener a low priority to prioritise handling of ongoing
    // requests over accepting new requests possibly overloading the system.
    let options = ActorOptions::default().with_priority(Priority::LOW);
    system_ref.try_spawn(ServerSupervisor(address), server, address, options)?;

    Ok(())
}

/// Our supervisor for the TCP server.
#[derive(Copy, Clone, Debug)]
struct ServerSupervisor(SocketAddr);

impl<S, NA> Supervisor<tcp::ServerSetup<S, NA>> for ServerSupervisor
where
    // Trait bounds needed by `tcp::ServerSetup`.
    S: Supervisor<NA> + Clone + 'static,
    NA: NewActor<Argument = (TcpStream, SocketAddr), Error = !> + Clone + 'static,
{
    fn decide(&mut self, err: tcp::ServerError<!>) -> SupervisorStrategy<SocketAddr> {
        use tcp::ServerError::*;
        match err {
            // When we hit an error accepting a connection we'll drop the old
            // listener and create a new one.
            Accept(err) => {
                error!("error accepting new connection: {}", err);
                SupervisorStrategy::Restart(self.0)
            }
            // Async function never return an error creating a new actor.
            NewActor(_) => unreachable!(),
        }
    }

    fn decide_on_restart_error(&mut self, err: io::Error) -> SupervisorStrategy<SocketAddr> {
        // If we can't create a new listener we'll stop.
        error!("error restarting the TCP server: {}", err);
        SupervisorStrategy::Stop
    }

    fn second_restart_error(&mut self, err: io::Error) {
        // This shouldn't ever be called as we don't restart the actor a second
        // time (see `decide_on_restart_error`), but just in case.
        error!("error restarting the actor a second time: {}", err);
    }
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
