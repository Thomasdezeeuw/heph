//! This is just a memory stress test of the runtime.
//!
//! Currently using 10 million "actors" this test uses 7.08 GB and takes ~26
//! seconds to spawn the actors.

#![feature(never_type)]

use log::info;

use heph::supervisor::NoSupervisor;
use heph::{actor, rt, ActorOptions, Runtime};

fn main() -> Result<(), rt::Error> {
    heph::log::init();
    Runtime::new()?
        .with_setup(move |mut runtime_ref| {
            const N: usize = 10_000_000;

            info!("Spawning {} actors, this might take a while", N);
            let start = std::time::Instant::now();
            for _ in 0..N {
                let actor = actor as fn(_) -> _;
                // Don't run the actors at that will remove them from memory.
                let options = ActorOptions::default().mark_not_ready();
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
        })
        .start()
}

/// Our "actor", but it doesn't do much.
async fn actor(_: actor::Context<!>) -> Result<(), !> {
    Ok(())
}

async fn control_actor(_: actor::Context<!>) -> Result<(), !> {
    info!("Running, check the memory usage!");
    info!("Send a signal (e.g. by pressing Ctrl-C) to stop.");
    Ok(())
}
