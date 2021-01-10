#![feature(never_type)]

use std::thread::sleep;
use std::time::Duration;

use log::warn;

use heph::actor_ref::{ActorRef, SendError};
use heph::supervisor::{NoSupervisor, SupervisorStrategy};
use heph::{actor, rt, ActorOptions, Runtime, RuntimeRef};

fn main() -> Result<(), rt::Error> {
    let mut runtime = Runtime::new()?.with_setup(setup);

    // Enable tracing of the runtime.
    runtime.enable_tracing("heph_tracing_example.bin.log")?;

    runtime.start()
}

const CHAIN_SIZE: usize = 5;

/// Setup function will start a chain of `relay_actors` and a final
/// `print_actor`, just to create some activity for the trace.
fn setup(mut runtime_ref: RuntimeRef) -> Result<(), !> {
    let print_actor = print_actor as fn(_) -> _;
    let relay_actor = relay_actor as fn(_, _) -> _;

    // Spawn our printing actor.
    let actor_ref = runtime_ref.spawn_local(NoSupervisor, print_actor, (), ActorOptions::default());

    // Create a chain of relay actors that will relay messages to the next
    // actor.
    let mut next_actor_ref = actor_ref;
    for _ in 0..CHAIN_SIZE {
        next_actor_ref = runtime_ref.spawn_local(
            |err| {
                warn!("error running actor: {}", err);
                SupervisorStrategy::Stop
            },
            relay_actor,
            next_actor_ref,
            ActorOptions::default(),
        );
    }

    // Send the messages down the chain of actors.
    let msgs = &[
        "First message: Hello World!",
        "Hello Mars!",
        "End of transmission.",
    ];
    for msg in msgs {
        next_actor_ref.try_send(*msg).unwrap();
    }

    Ok(())
}

/// Actor that relays all messages it receives to actor behind `relay`.
async fn relay_actor(
    mut ctx: actor::Context<&'static str>,
    relay: ActorRef<&'static str>,
) -> Result<(), SendError> {
    while let Ok(msg) = ctx.receive_next().await {
        // Sleep to extend the duration of the trace.
        sleep(Duration::from_millis(5));
        relay.send(msg).await?;
    }
    Ok(())
}

/// Actor that prints all messages it receives.
async fn print_actor(mut ctx: actor::Context<&'static str>) -> Result<(), !> {
    while let Ok(msg) = ctx.receive_next().await {
        // Sleep to extend the duration of the trace.
        sleep(Duration::from_millis(5));
        println!("Received message: {}", msg);
    }
    Ok(())
}
