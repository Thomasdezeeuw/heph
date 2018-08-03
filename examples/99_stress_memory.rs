//! This is just a memory stress test of the system.
//!
//! Currently using 10 million "actors" this test uses 1.68 GB.

#![feature(async_await, futures_api, never_type)]

use std::thread;
use std::time::Duration;

use heph::actor::{ActorContext, actor_factory};
use heph::system::{ActorSystemBuilder, ActorOptions};

/// Our "actor", but it doesn't do much.
async fn actor(_: ActorContext<!>, _: ()) -> Result<(), !> {
    Ok(())
}

fn main() {
    let mut actor_system = ActorSystemBuilder::default().build()
        .expect("unable to build the actor system");

    for _ in 0..10_000_000 {
        let new_actor = actor_factory(actor);
        let _ = actor_system.add_actor(new_actor, (), ActorOptions::default());
    }

    println!("Running, check the memory usage!");
    thread::sleep(Duration::from_secs(100));
}
