#![feature(never_type)]

use std::io;

use heph::actor::{self, actor_fn, Actor, NewActor};
use heph::supervisor::{Supervisor, SupervisorStrategy};
use heph_rt::net::{tcp, TcpStream};
use heph_rt::spawn::options::{ActorOptions, Priority};
use heph_rt::{self as rt, Runtime, ThreadLocal};
use log::{error, info};

fn main() -> Result<(), rt::Error> {
    // For this example we'll enable logging, this give us a bit more insight
    // into the runtime. By default it only logs informational or more severe
    // messages, the environment variable `LOG_LEVEL` can be set to change this.
    // For example enabling logging of trace severity message can be done by
    // setting `LOG_LEVEL=trace`.
    std_logger::Config::logfmt().init();

    // Create our TCP server. This server will create a new actor for each
    // incoming TCP connection. As always, actors needs supervision, this is
    // done by `conn_supervisor` in this example. And as each actor will need to
    // be added to the runtime it needs the `ActorOptions` to do that, we'll use
    // the defaults options here.
    let actor = actor_fn(conn_actor);
    let address = "127.0.0.1:7890".parse().unwrap();
    let server = tcp::server::setup(address, conn_supervisor, actor, ActorOptions::default())
        .map_err(rt::Error::setup)?;

    // Just like in examples 1 and 2 we'll create our runtime and run our setup
    // function. But for this example we'll create a worker thread per available
    // CPU core using `use_all_cores`.
    let mut runtime = Runtime::setup().use_all_cores().build()?;
    runtime.run_on_workers(move |mut runtime_ref| -> io::Result<()> {
        // As the TCP server is just another actor we need to spawn it like any
        // other actor. And again actors needs supervision, thus we provide
        // `ServerSupervisor` as supervisor.
        // We'll give our server a low priority to prioritise handling of
        // ongoing requests over accepting new requests possibly overloading the
        // system.
        let options = ActorOptions::default().with_priority(Priority::LOW);
        let server_ref = runtime_ref.spawn_local(ServerSupervisor, server, (), options);

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
    NA: NewActor<Argument = (), Error = !>,
    NA::Actor: Actor<Error = tcp::server::Error<!>>,
{
    fn decide(&mut self, err: tcp::server::Error<!>) -> SupervisorStrategy<()> {
        match err {
            // When we hit an error accepting a connection we'll drop the old
            // listener and create a new one.
            tcp::server::Error::Accept(err) => {
                error!("error accepting new connection: {err}");
                SupervisorStrategy::Restart(())
            }
            // Async function never return an error creating a new actor.
            tcp::server::Error::NewActor(err) => err,
        }
    }

    fn decide_on_restart_error(&mut self, err: !) -> SupervisorStrategy<()> {
        err
    }

    fn second_restart_error(&mut self, err: !) {
        err
    }
}

/// Our supervisor for the connection actor.
///
/// Since we can't create a new TCP connection all this supervisor does is log
/// the error and signal to stop the actor.
fn conn_supervisor(err: io::Error) -> SupervisorStrategy<TcpStream> {
    error!("error handling connection: {err}");
    SupervisorStrategy::Stop
}

/// Our connection actor.
///
/// This actor will not receive any message and thus uses `!` (the never type)
/// as message type.
async fn conn_actor(_: actor::Context<!, ThreadLocal>, stream: TcpStream) -> io::Result<()> {
    let address = stream.peer_addr()?;
    info!(address = log::as_display!(address); "accepted connection");

    // This will allocate a new string which isn't the most efficient way to do
    // this, but it's the easiest so we'll keep this for sake of example.
    let ip = address.ip().to_string();

    // Next we'll write the IP address to the connection.
    stream.send_all(ip).await?;
    Ok(())
}
