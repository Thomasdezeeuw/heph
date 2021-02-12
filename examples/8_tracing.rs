#![feature(never_type)]

use std::thread::sleep;
use std::time::Duration;

use log::warn;

use heph::actor::sync::SyncContext;
use heph::actor_ref::{ActorRef, SendError};
use heph::rt::options::SyncActorOptions;
use heph::supervisor::{NoSupervisor, SupervisorStrategy};
use heph::{actor, rt, ActorOptions, Runtime, RuntimeRef};

fn main() -> Result<(), rt::Error> {
    let mut runtime = Runtime::new()?;
    // Enable tracing of the runtime.
    runtime.enable_tracing("heph_tracing_example.bin.log")?;

    // Spawn our printing actor.
    // NOTE: to enable tracing for this sync actor it must be spawned after
    // enabling tracing.
    let options = SyncActorOptions::default().with_name("Printer".to_owned());
    let print_actor = print_actor as fn(_) -> _;
    let actor_ref = runtime.spawn_sync_actor(NoSupervisor, print_actor, (), options)?;

    runtime
        .with_setup(|runtime_ref| setup(runtime_ref, actor_ref))
        .start()
}

const CHAIN_SIZE: usize = 5;

/// Setup function will start a chain of `relay_actors`, just to create some
/// activity for the trace.
fn setup(mut runtime_ref: RuntimeRef, actor_ref: ActorRef<&'static str>) -> Result<(), !> {
    let relay_actor = relay_actor as fn(_, _) -> _;

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

/// Sync actor that prints all messages it receives.
fn print_actor(mut ctx: SyncContext<&'static str>) {
    loop {
        // Start timing of receiving a message.
        let timing = ctx.start_trace();
        let msg = if let Ok(msg) = ctx.receive_next() {
            msg
        } else {
            break;
        };
        // Finish timing.
        ctx.finish_trace(timing, "receiving message", &[]);

        let timing = ctx.start_trace();
        // Sleep to extend the duration of the trace.
        sleep(Duration::from_millis(5));
        println!("Received message: {}", msg);
        ctx.finish_trace(timing, "printing message", &[("message", &msg)]);
    }
}
