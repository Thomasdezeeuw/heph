//! When running this example example 5a must also run!

#![feature(never_type)]

use heph::net::rpc::{Remote, RemoteActors};
use heph::supervisor::NoSupervisor;
use heph::{actor, rt, ActorOptions, ActorRef, Runtime};

fn main() -> Result<(), rt::Error> {
    heph::log::init();

    // See examples 1 and 2 for detailed explanation of the runtime setup.
    let mut runtime = Runtime::new()?;

    // This address of the remote node.
    let remote_address = "127.0.0.1:9001".parse().unwrap();
    // Connect to the remote node.
    let remote_actors = RemoteActors::connect(&mut runtime, remote_address);
    // Create a reference to the registered greeter actor (see example 5a).
    let actor_ref = remote_actors.create_ref("greeter");

    runtime
        .with_setup(|mut runtime_ref| {
            let options = ActorOptions::default().mark_ready();
            let actor = greeter_actor as fn(_, _) -> _;
            runtime_ref.spawn_local(NoSupervisor, actor, actor_ref, options);
            Ok(())
        })
        .start()
}

async fn greeter_actor(_: actor::Context<!>, mut actor_ref: ActorRef<Remote>) -> Result<(), !> {
    // Send the actor a message.
    // Note that we can't reliably detect if the message is delivered, so when
    // waiting for a response its advisable to use a timeout of some kind.
    actor_ref <<= "Thomas";

    /* TODO: support RPC.
    // RPC is also supported.
    // Note that, much like with sending messages, we can't reliably detect if
    // the remote actor is going to respond. So its advisable to use a timeout
    // or similar mechanism to ensure this doesn't wait forever.
    match actor_ref.rpc(&mut ctx, "Bob") {
        Ok(response) => match response.await {
            Ok(response) => {
                info!("got a response: {}", response);
            }
            Err(_) => error!("remote actor didn't send a response"),
        },
        Err(err) => error!("failed to send message to remote actor"),
    }
    */

    // See example 5a for the actor that handles these messages.
    Ok(())
}
