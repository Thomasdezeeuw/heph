//! This is just a memory stress test of the runtime.
//!
//! Currently using 10 million "actors" this test uses 2.09 GB and takes ~5
//! seconds to spawn the actors.

#![feature(never_type)]

use log::info;

use heph::actor;
use heph::rt::{self, Runtime, ThreadLocal};
use heph::spawn::ActorOptions;
use heph::supervisor::NoSupervisor;

fn main() -> Result<(), rt::Error> {
    std_logger::init();
    let mut runtime = Runtime::setup().build()?;
    runtime.run_on_workers(move |mut runtime_ref| -> Result<(), !> {
        const N: usize = 10_000_000;

        info!("Spawning {} actors, this might take a while", N);
        let start = std::time::Instant::now();
        for _ in 0..N {
            let actor = actor as fn(_) -> _;
            // Don't run the actors as that will remove them from memory.
            let options = ActorOptions::default().mark_ready(false);
            runtime_ref.spawn_local(NoSupervisor, actor, (), options);
        }
        info!("Spawning took {:?}", start.elapsed());

        runtime_ref.spawn_local(
            NoSupervisor,
            control_actor as fn(_) -> _,
            (),
            ActorOptions::default(),
        );

        Ok(())
    })?;
    runtime.start()
}

/// Our "actor", but it doesn't do much.
async fn actor(_: actor::Context<!, ThreadLocal>) {
    /* Nothing. */
}

async fn control_actor(_: actor::Context<!, ThreadLocal>) {
    info!("Running, check the memory usage!");
    info!("Send a signal (e.g. by pressing Ctrl-C) to stop.");
}
