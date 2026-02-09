use heph::actor::{self, actor_fn};
use heph::supervisor::NoSupervisor;
use heph::sync;
use heph_rt::spawn::{ActorOptions, SyncActorOptions};
use heph_rt::{self as rt, Runtime, RuntimeRef, Sync, ThreadLocal, ThreadSafe, process};

// Heph has build-in support for handling process signals. This example shows
// this can be used to cleanly shutdown your application.
//
// Run using:
// $ cargo run --example 6_process_signals
//
// Then use ctrl-c (sending an interrupt signal) to stop all actors.
fn main() -> Result<(), rt::Error> {
    // Enable logging.
    std_logger::Config::logfmt().init();

    // Process signal handling is supported for all actors and its as simple as
    // receiving `process::Signal` messages from the runtime and handling them
    // accordingly in the actors.

    let mut runtime = Runtime::setup().build()?;

    // Receiving process signals for thread-local actors.
    runtime.run_on_workers(|mut runtime_ref: RuntimeRef| -> Result<(), rt::Error> {
        // Spawn our actor.
        let actor = actor_fn(thread_local_actor);
        let actor_ref = runtime_ref.spawn_local(NoSupervisor, actor, (), ActorOptions::default());
        // Send it a normal message.
        actor_ref.try_send("Hello thread local actor").unwrap();

        // Register our actor reference to receive process signals.
        runtime_ref.receive_signals(actor_ref.try_map());

        Ok(())
    })?;

    // Thread-safe actors, following the same pattern as thread-local actors.
    let actor = actor_fn(thread_safe_actor);
    let actor_ref = runtime.spawn(NoSupervisor, actor, (), ActorOptions::default());
    actor_ref.try_send("Hello thread safe actor").unwrap();
    runtime.receive_signals(actor_ref.try_map());

    // Synchronous actors, also following a similar pattern as for thread-local actors.
    let actor = actor_fn(sync_actor);
    let actor_ref =
        runtime.spawn_sync_actor(NoSupervisor, actor, (), SyncActorOptions::default())?;
    actor_ref.try_send("Hello sync actor").unwrap();
    runtime.receive_signals(actor_ref.try_map());

    runtime.start()
}

/// Message type our actors will receive, see the `From` and `TryFrom`
/// implementations below.
enum Message {
    /// Print the string.
    Print(String),
    /// Shutdown.
    Terminate,
}

impl From<&'static str> for Message {
    fn from(msg: &'static str) -> Message {
        Message::Print(msg.to_owned())
    }
}

impl TryFrom<process::Signal> for Message {
    type Error = ();

    fn try_from(signal: process::Signal) -> Result<Self, Self::Error> {
        if signal.should_exit() {
            Ok(Message::Terminate)
        } else {
            Err(())
        }
    }
}

async fn thread_local_actor(mut ctx: actor::Context<Message, ThreadLocal>) {
    while let Ok(msg) = ctx.receive_next().await {
        match msg {
            Message::Print(msg) => println!("Got a message: {msg}"),
            Message::Terminate => break,
        }
    }

    println!("shutting down the thread local actor");
}

async fn thread_safe_actor(mut ctx: actor::Context<Message, ThreadSafe>) {
    while let Ok(msg) = ctx.receive_next().await {
        match msg {
            Message::Print(msg) => println!("Got a message: {msg}"),
            Message::Terminate => break,
        }
    }

    println!("shutting down the thread safe actor");
}

fn sync_actor(mut ctx: sync::Context<Message, Sync>) {
    while let Ok(msg) = ctx.receive_next() {
        match msg {
            Message::Print(msg) => println!("Got a message: {msg}"),
            Message::Terminate => break,
        }
    }

    println!("shutting down the synchronous actor");
}
