//! This is just a memory stress test of the runtime.
//!
//! Currently using 10 million "actors" this test uses ~3 GB and takes ~3
//! seconds to spawn the actors.

#![feature(never_type)]

use std::future::pending;

use heph::actor::{self, actor_fn};
use heph::supervisor::NoSupervisor;
use heph_rt::spawn::ActorOptions;
use heph_rt::{self as rt, Runtime, ThreadLocal};
use log::info;

fn main() -> Result<(), rt::Error> {
    // Enable logging.
    std_logger::Config::logfmt().init();

    let mut runtime = Runtime::setup().build()?;
    runtime.run_on_workers(move |mut runtime_ref| -> Result<(), !> {
        const N: usize = 10_000_000;

        info!("Spawning {N} actors, this might take a while");
        let start = std::time::Instant::now();
        let actor = actor_fn(actor);
        for _ in 0..N {
            runtime_ref.spawn_local(NoSupervisor, actor, (), ActorOptions::default());
        }
        info!("Spawning took {:?}", start.elapsed());

        let control_actor = actor_fn(control_actor);
        runtime_ref.spawn_local(NoSupervisor, control_actor, (), ActorOptions::default());

        Ok(())
    })?;
    runtime.start()
}

/// Our "actor", but it doesn't do much.
async fn actor(_: actor::Context<!, ThreadLocal>) {
    pending().await
}

async fn control_actor(_: actor::Context<!, ThreadLocal>) {
    info!("Running, check the memory usage!");
    info!("Send a signal (e.g. by pressing Ctrl-C) to stop.");
}
