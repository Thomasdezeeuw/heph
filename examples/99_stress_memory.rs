//! This is just a memory stress test of the system.
//!
//! Currently using 10 million "actors" this test uses 1.74 GB.

#![feature(never_type)]

use std::thread;
use std::time::Duration;

use heph::supervisor::NoSupervisor;
use heph::{actor, ActorOptions, Runtime, RuntimeError};

fn main() -> Result<(), RuntimeError> {
    Runtime::new()
        .with_setup(|mut system_ref| {
            let actor = actor as fn(_) -> _;
            for _ in 0..10_000_000 {
                system_ref.spawn(NoSupervisor, actor, (), ActorOptions::default());
            }

            println!("Running, check the memory usage!");
            thread::sleep(Duration::from_secs(100));

            Ok(())
        })
        .run()
}

/// Our "actor", but it doesn't do much.
async fn actor(_: actor::Context<!>) -> Result<(), !> {
    Ok(())
}
