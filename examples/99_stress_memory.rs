//! This is just a memory stress test of the runtime.
//!
//! Currently using 10 million "actors" this test uses 2.27 GB.

#![feature(never_type)]

use std::thread;
use std::time::Duration;

use heph::supervisor::NoSupervisor;
use heph::{actor, ActorOptions, Runtime, RuntimeError};

fn main() -> Result<(), RuntimeError> {
    let start = std::time::Instant::now();
    Runtime::new()
        .with_setup(move |mut runtime_ref| {
            let actor = actor as fn(_) -> _;
            for _ in 0..10_000_000 {
                runtime_ref.spawn(NoSupervisor, actor, (), ActorOptions::default());
            }

            println!("Elapsed: {:?}", start.elapsed());

            println!("Running, check the memory usage!");
            thread::sleep(Duration::from_secs(100));

            Ok(())
        })
        .start()
}

/// Our "actor", but it doesn't do much.
async fn actor(_: actor::Context<!>) -> Result<(), !> {
    Ok(())
}
