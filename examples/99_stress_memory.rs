//! This is just a memory stress test of the runtime.
//!
//! Currently using 10 million "actors" this test uses 6.81 GB.

#![feature(never_type)]

use std::thread;
use std::time::Duration;

use log::info;

use heph::supervisor::NoSupervisor;
use heph::{actor, rt, ActorOptions, Runtime};

fn main() -> Result<(), rt::Error> {
    heph::log::init();
    let start = std::time::Instant::now();
    Runtime::new()?
        .with_setup(move |mut runtime_ref| {
            let actor = actor as fn(_) -> _;
            for _ in 0..10_000_000 {
                runtime_ref.spawn_local(NoSupervisor, actor, (), ActorOptions::default());
            }

            let sleepy_actor = sleepy_actor as fn(_) -> _;
            runtime_ref.spawn_local(
                NoSupervisor,
                sleepy_actor,
                (),
                ActorOptions::default().mark_ready(),
            );

            info!("Elapsed: {:?}", start.elapsed());

            Ok(())
        })
        .start()
}

/// Our "actor", but it doesn't do much.
async fn actor(_: actor::Context<!>) -> Result<(), !> {
    Ok(())
}

async fn sleepy_actor(_: actor::Context<!>) -> Result<(), !> {
    info!("Running, check the memory usage!");
    thread::sleep(Duration::from_secs(5));
    Ok(())
}
