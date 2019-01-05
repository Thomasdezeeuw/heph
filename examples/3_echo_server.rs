#![feature(async_await, await_macro, futures_api, never_type)]

use std::io;
use std::default::Default;
use std::net::SocketAddr;

use futures_util::{AsyncReadExt, TryFutureExt};
use log::{error, info};

use heph::actor::ActorContext;
use heph::log::REQUEST_TARGET;
use heph::net::{TcpListener, TcpStream};
use heph::supervisor::{SupervisorStrategy, NoSupervisor};
use heph::system::{ActorSystem, ActorSystemRef, ActorOptions, InitiatorOptions, RuntimeError};

/// This is our setup function that will add the count actor to the actor
/// system, much like the `add_greeter_actor` in example 1.
fn add_count_actor(mut system_ref: ActorSystemRef) -> io::Result<()> {
    // Just like in example 1 we'll add our actor to the system.
    system_ref.spawn(NoSupervisor, count_actor as fn(_) -> _, (),
        ActorOptions {
            // But this example we'll use the `register` option. This registers
            // the actor in the actor registry and allows it to be looked up,
            // see the `echo_actor` below.
            //
            // Note: this registry is per thread!
            register: true,
            .. Default::default()
        });

    Ok(())
}

/// Message type used by `count_actor`.
struct Add;

/// Our actor that receives addition messages and it to the total count.
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

/// Our actor that handles the incoming TCP connections.
async fn echo_actor(mut ctx: ActorContext<!>, stream: TcpStream, address: SocketAddr) -> io::Result<()> {
    // Here we use a special request target to mark this log as a request. This
    // will cause it to be printed to standard out, rather then standard error.
    info!(target: REQUEST_TARGET, "accepted connection: address={}", address);

    // Next we'll lookup the `count_actor` in the registry.
    //
    // Unfortunately functions can be passed as a type, i.e. we can't call
    // `loopup::<count_actor>()`. To overcome this we need actually pass a
    // reference to get the type information and pass that to the
    // `loopup_untyped` function.
    if let Some(mut count_actor_ref) = ctx.system_ref().lookup_val(&(count_actor as fn(_) -> _)) {
        // If we can find the actor we'll send it a message to add to the total.
        let _ = count_actor_ref.send(Add);
    }
    // If we can't find the count actor, we can continue our work without
    // sending a message.

    let (mut read, mut write) = stream.split();
    await!(read.copy_into(&mut write).map_ok(|_| ()))
}

// Our supervisor, same as in example 2.
fn echo_supervisor(err: io::Error) -> SupervisorStrategy<(TcpStream, SocketAddr)> {
    error!("error handling connection: {}", err);
    SupervisorStrategy::Stop
}

// Main is mostly the same as example 2, only here we add a setup function
// (already seen in example 1) and set the number of threads to 2 to indicate
// the 1 actor per thread problem quicker (hopefully).
fn main() -> Result<(), RuntimeError> {
    heph::log::init();

    let address = "127.0.0.1:7890".parse().unwrap();
    let listener = TcpListener::bind(address, echo_supervisor, echo_actor as fn(_, _, _) -> _, ActorOptions::default())
        .expect("unable to bind TCP listener");
    info!("listening: address={}", address);

    ActorSystem::new()
        .with_setup(add_count_actor)
        .with_initiator(listener, InitiatorOptions::default())
        .num_threads(2)
        .run()
}
