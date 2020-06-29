#![feature(never_type)]

use std::convert::TryFrom;

use heph::actor;
use heph::actor::context::ThreadSafe;
use heph::actor::sync::SyncContext;
use heph::rt::{self, ActorOptions, Runtime, RuntimeRef, Signal};
use heph::supervisor::NoSupervisor;

fn main() -> Result<(), rt::Error> {
    let mut runtime = Runtime::new()?;

    // Signal handling is support for all actor and its as simple as receiving
    // `Signal` messages from the runtime and handling them accordingly in the
    // actors.

    // Synchronous actor.
    // Spawn our actor.
    let sync_actor = sync_actor as fn(_) -> _;
    let mut actor_ref = runtime.spawn_sync_actor(NoSupervisor, sync_actor, ())?;
    // Send it message
    actor_ref <<= "Hello sync actor";
    // Register our actor reference to receive process signals.
    runtime.receive_signals(actor_ref.try_map());

    // Thread-safe actor (following the same structure as synchronous actors).
    let thread_safe_actor = thread_safe_actor as fn(_) -> _;
    let mut actor_ref = runtime.spawn(NoSupervisor, thread_safe_actor, (), ActorOptions::default());
    actor_ref <<= "Hello thread safe actor";
    runtime.receive_signals(actor_ref.try_map());

    // Thread-local actor (following the same structure as synchronous actors).
    let setup = |mut runtime_ref: RuntimeRef| {
        let local_actor = local_actor as fn(_) -> _;
        let mut actor_ref =
            runtime_ref.spawn_local(NoSupervisor, local_actor, (), ActorOptions::default());
        actor_ref <<= "Hello thread local actor";
        runtime_ref.receive_signals(actor_ref.try_map());

        Ok(())
    };

    runtime.with_setup(setup).start()
}

enum Message {
    /// Print the string.
    Print(String),
    /// Shutdown
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
        }
    }
}

fn sync_actor(mut ctx: SyncContext<Message>) -> Result<(), !> {
    while let Ok(msg) = ctx.receive_next() {
        match msg {
            Message::Print(msg) => println!("Got a message: {}", msg),
            Message::Terminate => break,
        }
    }

    println!("shutting down the synchronous actor");
    Ok(())
}

async fn thread_safe_actor(mut ctx: actor::Context<Message, ThreadSafe>) -> Result<(), !> {
    loop {
        match ctx.receive_next().await {
            Message::Print(msg) => println!("Got a message: {}", msg),
            Message::Terminate => break,
        }
    }

    println!("shutting down the thread local actor");
    Ok(())
}

async fn local_actor(mut ctx: actor::Context<Message>) -> Result<(), !> {
    loop {
        match ctx.receive_next().await {
            Message::Print(msg) => println!("Got a message: {}", msg),
            Message::Terminate => break,
        }
    }

    println!("shutting down the thread safe actor");
    Ok(())
}
