//! When running this example example 5a must also run!

#![feature(never_type)]

use std::io;

use heph::net::relay::{Drop, RemoteRelay};
use heph::{rt, ActorOptions, ActorRef, Runtime};
use log::error;

fn main() -> Result<(), rt::Error<io::Error>> {
    heph::log::init();

    // Reverse of example 5a.
    let local_address = "127.0.0.1:9002".parse().unwrap();
    let remote_address = "127.0.0.1:9001".parse().unwrap();

    let mut runtime = Runtime::new().map_err(rt::Error::map_type)?;

    // Spawn our remote relay actor.
    let options = ActorOptions::default();
    let mut remote_relay =
        RemoteRelay::bind::<_, String>(&mut runtime, local_address, Drop, options)?;

    let remote_actor_ref: ActorRef<String> = remote_relay.create_ref(remote_address);
    if let Err(err) = remote_actor_ref.send("Thomas".to_owned()) {
        error!("error sending message: {}", err);
    }

    runtime.start().map_err(rt::Error::map_type)
}
