//! This is just a memory stress test of the system.
//!
//! Currently using 10 million "actors" this test uses 2.19 GB.

#![feature(async_await, futures_api, never_type)]

use std::thread;
use std::time::Duration;

use heph::actor::ActorContext;
use heph::supervisor::NoopSupervisor;
use heph::system::{ActorSystem, ActorOptions, RuntimeError};

/// Our "actor", but it doesn't do much.
async fn actor(_: ActorContext<!>) -> Result<(), !> {
    Ok(())
}

fn main() -> Result<(), RuntimeError> {
    ActorSystem::new()
        .with_setup(|mut system_ref| {
            for _ in 0..10_000_000 {
                let _ = system_ref.spawn(NoopSupervisor, actor as fn(_) -> _, (), ActorOptions::default());
            }

            println!("Running, check the memory usage!");
            thread::sleep(Duration::from_secs(100));

            Ok(())
        })
        .run()
}
