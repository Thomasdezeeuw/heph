#![feature(async_await, await_macro, futures_api, never_type)]

use std::io;
use std::default::Default;
use std::net::SocketAddr;

use futures_util::{AsyncReadExt, TryFutureExt};

use heph::actor::ActorContext;
use heph::log::{self, error, info};
use heph::net::{TcpListener, TcpStream};
use heph::supervisor::{SupervisorStrategy, NoSupervisor};
use heph::system::options::Priority;
use heph::system::{ActorSystem, ActorSystemRef, ActorOptions, RuntimeError};

// Main is mostly the same as example 2, but we only use 2 worker threads.
fn main() -> Result<(), RuntimeError<io::Error>> {
    log::init();

    ActorSystem::new()
        .with_setup(setup)
        .num_threads(2)
        .run()
}

/// This is our setup function that will add the TCP listener to the actor
/// system, much like in example 2, and add the count actor to the actor system,
/// much like the `add_greeter_actor` in example 1.
fn setup(mut system_ref: ActorSystemRef) -> io::Result<()> {
    // Just like in example 2 we'll add our TCP listener to the system.
    let actor = echo_actor as fn(_, _, _) -> _;
    let listener = TcpListener::new(echo_supervisor, actor, ActorOptions::default());
    let address = "127.0.0.1:7890".parse().unwrap();
    system_ref.spawn(listener_supervisor, listener, address, ActorOptions {
        priority: Priority::LOW,
        .. Default::default()
    })?;

    // In this example we'll use the Actor Registry. This also actors to be
    // lookup dynamically at runtime, see the `echo_actor` for an example of
    // that.
    system_ref.spawn(NoSupervisor, count_actor as fn(_) -> _, (), ActorOptions {
        // To add the actor to the Actor Registry we simply set the `register`
        // option. This registers the actor in the Actor Registry and allows it
        // to be looked up, see the `echo_actor` below.
        //
        // Note: this registry is per thread!
        register: true,
        .. Default::default()
    }).unwrap();

    Ok(())
}

/// Our supervisor for the TCP listener, same as in example 2.
fn listener_supervisor(err: io::Error) -> SupervisorStrategy<(SocketAddr)> {
    error!("error in TCP listener: {}", err);
    SupervisorStrategy::Stop
}

/// Message type used by `count_actor`.
struct Add;

/// Our actor that receives `Add`ition messages and it to the total count.
///
/// Note: that 1 actor per thread is started in the setup function. Which means
/// that if you run this example it could display a total count of 1 twice. This
/// means that thread 1 handled the first request and thread 2 handled the
/// second, be careful of this when implementing a counter this way.
async fn count_actor(mut ctx: ActorContext<Add>) -> Result<(), !> {
    let mut total = 0;
    loop {
        let _msg = await!(ctx.receive());
        total += 1;

        println!("Total count: {}", total);
    }
}

/// Our supervisor for the connection actor, same as in example 2.
fn echo_supervisor(err: io::Error) -> SupervisorStrategy<(TcpStream, SocketAddr)> {
    error!("error handling connection: {}", err);
    SupervisorStrategy::Stop
}

/// Our actor that handles the incoming TCP connections.
async fn echo_actor(mut ctx: ActorContext<!>, stream: TcpStream, address: SocketAddr) -> io::Result<()> {
    // Here we use a special request target to mark this log as a request. This
    // will cause it to be printed to standard out, rather then standard error.
    info!(target: log::REQUEST_TARGET, "accepted connection: address={}", address);

    // Next we'll lookup the `count_actor` in the registry.
    //
    // Unfortunately functions can't be passed as a type, thus we can't call
    // `loopup::<count_actor>()`. To overcome this we need actually pass a
    // reference to get the type information and pass that to the
    // `lookup_untyped` function.
    //
    // Both `lookup` and `lookup_untyped` do the same thing; look up a
    // registered actor in the Actor Registry and return an actor reference to
    // that actor.
    if let Some(mut count_actor_ref) = ctx.system_ref().lookup_val(&(count_actor as fn(_) -> _)) {
        // If we can find the actor we'll send it a message to add to the total.
        let _ = count_actor_ref.send(Add);
    }
    // If we can't find the count actor, we can continue our work without
    // sending a message.

    // Here we'll split the TCP stream and copy every thing back, this is part
    // of the `AsyncReadExt` trait.
    let (mut read, mut write) = stream.split();
    await!(read.copy_into(&mut write).map_ok(|_| ()))
}
