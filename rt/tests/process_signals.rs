#![feature(async_iterator, never_type, write_all_vectored)]

use std::future::pending;
use std::process;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use heph::actor::{self, actor_fn};
use heph::supervisor::NoSupervisor;
use heph::sync;
use heph_rt::spawn::options::{ActorOptions, FutureOptions, SyncActorOptions};
use heph_rt::{Runtime, Signal};

#[path = "util/mod.rs"] // rustfmt can't find the file.
#[macro_use]
mod util;

use util::send_signal;

fn main() {
    no_signal_handlers();
    with_signal_handles();
}

/// Runtime without any actor to receive the signal should stop itself.
fn no_signal_handlers() {
    let mut runtime = Runtime::setup().build().unwrap();
    // We add a never ending process to make sure that the runtime doesn't
    // shutdown because no processes are left to run.
    runtime.spawn_future(pending(), FutureOptions::default());
    send_signal(process::id(), libc::SIGINT).expect("failed to send signal");
    let err_str = runtime.start().unwrap_err().to_string();
    assert!(
        err_str.contains("received process signal, but no receivers for it: stopping runtime"),
        "got error '{err_str}'",
    );
}

/// Runtime with actors to receive the signal should not stop and relay the
/// signals to the actors instead.
fn with_signal_handles() {
    let mut runtime = Runtime::setup().build().unwrap();

    let thread_local = Arc::new(AtomicUsize::new(0));
    let thread_safe1 = Arc::new(AtomicUsize::new(0)); // Via `RuntimeRef`.
    let thread_safe2 = Arc::new(AtomicUsize::new(0)); // Via `Runtime`.
    let sync = Arc::new(AtomicUsize::new(0));

    let tl = thread_local.clone();
    let ts = thread_safe1.clone();
    runtime
        .run_on_workers(|mut runtime_ref| -> Result<(), !> {
            let tla = actor_fn(actor);
            let actor_ref = runtime_ref.spawn_local(NoSupervisor, tla, tl, ActorOptions::default());
            runtime_ref.receive_signals(actor_ref);

            let tsa = actor_fn(actor);
            let actor_ref = runtime_ref.spawn(NoSupervisor, tsa, ts, ActorOptions::default());
            runtime_ref.receive_signals(actor_ref);

            Ok(())
        })
        .unwrap();

    let actor_ref = runtime.spawn(
        NoSupervisor,
        actor_fn(actor),
        thread_safe2.clone(),
        ActorOptions::default(),
    );
    runtime.receive_signals(actor_ref);

    let actor_ref = runtime
        .spawn_sync_actor(
            NoSupervisor,
            actor_fn(sync_actor),
            sync.clone(),
            SyncActorOptions::default(),
        )
        .unwrap();
    runtime.receive_signals(actor_ref);

    // Sending a signal now shouldn't cause the runtime to return an error (as
    // the signal is handled by one or more actors).
    send_signal(process::id(), libc::SIGINT).expect("failed to send signal");
    runtime.start().unwrap();

    // Make sure that all the actor received the signal once.
    assert_eq!(thread_local.load(Ordering::SeqCst), 1);
    assert_eq!(thread_safe1.load(Ordering::SeqCst), 1);
    assert_eq!(thread_safe2.load(Ordering::SeqCst), 1);
    assert_eq!(sync.load(Ordering::SeqCst), 1);
}

async fn actor<RT>(mut ctx: actor::Context<Signal, RT>, got_signal: Arc<AtomicUsize>) {
    let _msg = ctx.receive_next().await.unwrap();
    got_signal.fetch_add(1, Ordering::SeqCst);
}

fn sync_actor<RT>(mut ctx: sync::Context<Signal, RT>, got_signal: Arc<AtomicUsize>) {
    let _msg = ctx.receive_next().unwrap();
    got_signal.fetch_add(1, Ordering::SeqCst);
}
