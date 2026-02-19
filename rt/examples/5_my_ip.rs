#![feature(never_type)]

use std::io;
use std::net::SocketAddr;

use heph::actor::{self, Actor, NewActor, actor_fn};
use heph::supervisor::{Supervisor, SupervisorStrategy};
use heph_rt::fd::AsyncFd;
use heph_rt::net::{ServerError, TcpServer};
use heph_rt::spawn::options::{ActorOptions, Priority};
use heph_rt::{self as rt, Runtime, ThreadLocal};
use log::{error, info};

// This example shows a simple TCP server that writes the IP address of the
// connection to the connection.
//
// Run using:
// $ cargo run --example 5_my_ip
// To connect:
// $ nc localhost 7890
//
// To stop the server use ctrl-c.
fn main() -> Result<(), rt::Error> {
    // Enable logging.
    std_logger::Config::logfmt().init();

    // Create our TCP server. This server will create a new actor for each
    // incoming TCP connection.
    let actor = actor_fn(conn_actor);
    let address = "127.0.0.1:7890".parse().unwrap();
    let server = TcpServer::new(address, conn_supervisor, actor, ActorOptions::default())
        .map_err(rt::Error::setup)?;

    // Like in example 1 we'll create our runtime and run our setup function.
    // But we'll create a worker thread per available CPU core using
    // `use_all_cores`.
    let mut runtime = Runtime::setup().use_all_cores().build()?;
    runtime.run_on_workers(move |mut runtime_ref| -> Result<(), rt::Error> {
        // As the TCP server is just another actor we need to spawn it like any
        // other actor.
        // We'll give our server a low priority to prioritise handling of
        // ongoing requests over accepting new requests possibly overloading the
        // system.
        let options = ActorOptions::default().with_priority(Priority::LOW);
        let server_ref = runtime_ref
            .try_spawn_local(ServerSupervisor, server, (), options)
            .map_err(rt::Error::setup)?;

        // The server can handle the interrupt, terminate and quit signals,
        // so it will perform a clean shutdown for us.
        runtime_ref.receive_signals(server_ref.try_map());
        Ok(())
    })?;
    info!("listening on {address}");
    runtime.start()
}

/// Our supervisor for the TCP server.
#[derive(Copy, Clone, Debug)]
struct ServerSupervisor;

impl<NA> Supervisor<NA> for ServerSupervisor
where
    NA: NewActor<Argument = (), Error = io::Error>,
    NA::Actor: Actor<Error = ServerError<!>>,
{
    fn decide(&mut self, err: ServerError<!>) -> SupervisorStrategy<()> {
        match err {
            // When we hit an error accepting a connection we'll drop the old
            // listener and create a new one.
            ServerError::Accept(err) => {
                error!("error accepting new connection: {err}");
                SupervisorStrategy::Restart(())
            }
            // Async function never return an error creating a new actor.
            ServerError::NewActor(err) => err,
        }
    }

    fn decide_on_restart_error(&mut self, err: io::Error) -> SupervisorStrategy<()> {
        error!("failed to restart listener, trying again: {err}");
        SupervisorStrategy::Restart(())
    }

    fn second_restart_error(&mut self, err: io::Error) {
        error!("failed to restart listener a second time, stopping it: {err}");
    }
}

/// Our supervisor for the connection actor.
///
/// Since we can't create a new TCP connection all this supervisor does is log
/// the error and signal to stop the actor.
fn conn_supervisor(err: io::Error) -> SupervisorStrategy<AsyncFd> {
    error!("error handling connection: {err}");
    SupervisorStrategy::Stop
}

/// Our connection actor.
///
/// This actor will not receive any message and thus uses `!` (the never type)
/// as message type.
async fn conn_actor(_: actor::Context<!, ThreadLocal>, stream: AsyncFd) -> io::Result<()> {
    let address: SocketAddr = stream.peer_addr().await?;
    info!(address:%; "accepted connection");

    // This will allocate a new string which isn't the most efficient way to do
    // this, but it's the easiest so we'll keep this simple for sake of example.
    let ip = address.ip().to_string();

    // Next we'll write the IP address to the connection.
    stream.send_all(ip).await?;
    Ok(())
}
