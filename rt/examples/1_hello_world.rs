use std::io;

use heph::actor::{self, actor_fn};
use heph::supervisor::StopSupervisor;
use heph_rt::io::stdout;
use heph_rt::spawn::ActorOptions;
use heph_rt::{self as rt, Access, Runtime, RuntimeRef, ThreadLocal};

// Conforming to the tradition that is "Hello, World!", a simple program that
// prints "Hello, World!" from an actor.
//
// Run using:
// $ cargo run --example 1_hello_world
fn main() -> Result<(), rt::Error> {
    // Enable logging.
    std_logger::Config::logfmt().init();

    // We create a new runtime. Add a setup function, which adds our greeter
    // actor. And finally we start it.
    let mut runtime = Runtime::setup().build()?;
    runtime.run_on_workers(add_greeter_actor)?;
    runtime.start()
}

/// The is the setup function used in the runtime.
fn add_greeter_actor(mut runtime_ref: RuntimeRef) -> Result<(), rt::Error> {
    // We spawn our actor here. For more information on spawning actors see
    // example 2_spawning_actors.
    let actor = actor_fn(greeter_actor);
    runtime_ref.spawn_local(StopSupervisor, actor, (), ActorOptions::default());

    Ok(())
}

/// Our greeter actor.
///
/// It will print a greeter and stop.
async fn greeter_actor(ctx: actor::Context<(), ThreadLocal>) -> io::Result<()> {
    stdout(ctx.runtime_ref().sq())
        .write_all("Hello, World!\n")
        .await
}
