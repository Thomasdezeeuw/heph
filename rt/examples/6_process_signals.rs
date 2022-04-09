#![feature(never_type)]

use std::convert::TryFrom;

use heph_rt::actor::{self, SyncContext};
use heph_rt::rt::{self, Runtime, RuntimeRef, Signal, ThreadLocal, ThreadSafe};
use heph_rt::spawn::{ActorOptions, SyncActorOptions};
use heph_rt::supervisor::NoSupervisor;

fn main() -> Result<(), rt::Error> {
    // Signal handling is support for all actor and its as simple as receiving
    // `Signal` messages from the runtime and handling them accordingly in the
    // actors.

    let mut runtime = Runtime::setup().build()?;

    // Thread-local actors.
    runtime.run_on_workers(|mut runtime_ref: RuntimeRef| -> Result<(), !> {
        // Spawn our actor.
        let local_actor = local_actor as fn(_) -> _;
        let actor_ref =
            runtime_ref.spawn_local(NoSupervisor, local_actor, (), ActorOptions::default());
        actor_ref.try_send("Hello thread local actor").unwrap();
        // Register our actor reference to receive process signals.
        runtime_ref.receive_signals(actor_ref.try_map());

        Ok(())
    })?;

    // Synchronous actors (following the same structure as thread-local actors).
    let sync_actor = sync_actor as fn(_);
    let actor_ref =
        runtime.spawn_sync_actor(NoSupervisor, sync_actor, (), SyncActorOptions::default())?;
    actor_ref.try_send("Hello sync actor").unwrap();
    runtime.receive_signals(actor_ref.try_map());

    // Thread-safe actor (following the same structure as thread-local actors).
    let thread_safe_actor = thread_safe_actor as fn(_) -> _;
    let actor_ref = runtime.spawn(NoSupervisor, thread_safe_actor, (), ActorOptions::default());
    actor_ref.try_send("Hello thread safe actor").unwrap();
    runtime.receive_signals(actor_ref.try_map());

    runtime.start()
}

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

impl TryFrom<Signal> for Message {
    type Error = ();

    fn try_from(signal: Signal) -> Result<Self, Self::Error> {
        match signal {
            Signal::Interrupt | Signal::Terminate | Signal::Quit => Ok(Message::Terminate),
            _ => Err(()),
        }
    }
}

fn sync_actor(mut ctx: SyncContext<Message>) {
    while let Ok(msg) = ctx.receive_next() {
        match msg {
            Message::Print(msg) => println!("Got a message: {}", msg),
            Message::Terminate => break,
        }
    }

    println!("shutting down the synchronous actor");
}

async fn thread_safe_actor(mut ctx: actor::Context<Message, ThreadSafe>) {
    while let Ok(msg) = ctx.receive_next().await {
        match msg {
            Message::Print(msg) => println!("Got a message: {}", msg),
            Message::Terminate => break,
        }
    }

    println!("shutting down the thread safe actor");
}

async fn local_actor(mut ctx: actor::Context<Message, ThreadLocal>) {
    while let Ok(msg) = ctx.receive_next().await {
        match msg {
            Message::Print(msg) => println!("Got a message: {}", msg),
            Message::Terminate => break,
        }
    }

    println!("shutting down the thread local actor");
}
