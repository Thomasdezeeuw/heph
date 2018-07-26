//! This is just a memory stress test of the system.
//!
//! Currently using 10 million "actors" this test uses 2.60 GB.

#![feature(never_type)]

use std::thread;
use std::time::Duration;

use actor::actor::{Status, actor_fn};
use actor::system::{ActorSystemBuilder, ActorOptions};

fn main() {
    let mut actor_system = ActorSystemBuilder::default().build()
        .expect("unable to build the actor system");

    for _ in 0..10_000_000 {
        let actor = actor_fn(|_, _: !| -> Result<Status, !> {
            Ok(Status::Ready)
        });

        let _ = actor_system.add_actor(actor, ActorOptions::default());
    }

    println!("Running, check the memory usage!");
    thread::sleep(Duration::from_secs(100));
}
