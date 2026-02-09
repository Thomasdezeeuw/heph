use std::thread::sleep;
use std::time::Duration;

use heph::actor::{self, actor_fn};
use heph::actor_ref::{ActorRef, SendError};
use heph::supervisor::{NoSupervisor, SupervisorStrategy};
use heph::sync;
use heph_rt::spawn::{ActorOptions, SyncActorOptions};
use heph_rt::trace::Trace;
use heph_rt::{self as rt, Runtime, RuntimeRef};
use log::warn;

// Heph supports generating trace files in its own custom format, described in
// the [Trace Format design document]. This format can be converted into
// [Chrome's Trace Event Format] so it can be opened by [Catapult trace view],
// which is hosted at <https://chromium.googlesource.com/catapult>.
//
// Running this example is a little more complex than other examples:
// 1. Run the example, to generate the trace.
//    $ cargo run --example 8_tracing
// 2. Got into the tools directory.
//    $ cd ../tools
// 3. Convert the trace to Chrome's format.
//    $ cargo run --bin convert_trace ../rt/heph_tracing_example.bin.log
// 4. Make the trace viewable in HTML.
//    $ cd ../rt
//    $ $(CATAPULT_REPO)/tracing/bin/trace2html heph_tracing_example.json
// 5. Finally open the trace in your browser.
//    $ open heph_tracing_example.html
//
// [Trace Format design document]: ../doc/Trace%20Format.md
// [Chrome's Trace Event Format]: https://docs.google.com/document/d/1CvAClvFfyA5R-PhYUmn5OOQtYMH4h6I0nSsKchNAySU/preview
// [Catapult trace view]: https://chromium.googlesource.com/catapult/+/refs/heads/master/tracing/README.md
fn main() -> Result<(), rt::Error> {
    std_logger::Config::logfmt().init();

    let mut runtime_setup = Runtime::setup();
    // We need to explicitly enabling tracing and write it to a given path.
    runtime_setup.enable_tracing("heph_tracing_example.bin.log")?;
    let mut runtime = runtime_setup.build()?;

    // Spawn our printing actor.
    // NOTE: to enable tracing for this sync actor it must be spawned after
    // enabling tracing.
    let options = SyncActorOptions::default().with_thread_name("Printer".to_owned());
    let print_actor = actor_fn(print_actor);
    let actor_ref = runtime.spawn_sync_actor(NoSupervisor, print_actor, (), options)?;

    runtime.run_on_workers(|runtime_ref| setup(runtime_ref, actor_ref))?;

    runtime.start()
}

const CHAIN_SIZE: usize = 5;

/// Setup function will start a chain of `relay_actors`, just to create some
/// activity for the trace.
fn setup(mut runtime_ref: RuntimeRef, actor_ref: ActorRef<&'static str>) -> Result<(), rt::Error> {
    // Create a chain of relay actors that will relay messages to the next
    // actor.
    let mut next_actor_ref = actor_ref;
    for _ in 0..CHAIN_SIZE {
        let relay_actor = actor_fn(relay_actor);
        next_actor_ref = runtime_ref.spawn_local(
            |err| {
                warn!("error running actor: {err}");
                SupervisorStrategy::Stop
            },
            relay_actor,
            next_actor_ref,
            ActorOptions::default(),
        );
    }

    // The first actor in the chain will be a thread-safe actor.
    next_actor_ref = runtime_ref.spawn(
        |err| {
            warn!("error running actor: {err}");
            SupervisorStrategy::Stop
        },
        actor_fn(relay_actor),
        next_actor_ref,
        ActorOptions::default(),
    );

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
async fn relay_actor<RT>(
    mut ctx: actor::Context<&'static str, RT>,
    relay: ActorRef<&'static str>,
) -> Result<(), SendError>
where
    RT: rt::Access,
{
    let mut receive_timing = ctx.start_trace();
    while let Ok(msg) = ctx.receive_next().await {
        ctx.finish_trace(receive_timing, "receiving message", &[]);

        let send_timing = ctx.start_trace();
        // Sleep to extend the duration of the trace.
        sleep(Duration::from_millis(5));
        relay.send(msg).await?;
        ctx.finish_trace(send_timing, "sending message", &[]);

        receive_timing = ctx.start_trace();
    }
    Ok(())
}

/// Sync actor that prints all messages it receives.
fn print_actor(mut ctx: sync::Context<&'static str, rt::Sync>) {
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
        println!("Received message: {msg}");
        ctx.finish_trace(timing, "printing message", &[("message", &msg)]);
    }
}
