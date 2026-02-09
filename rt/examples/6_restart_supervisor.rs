use heph::actor::{self, actor_fn};
use heph::{restart_supervisor, sync};
use heph_rt::spawn::{ActorOptions, SyncActorOptions};
use heph_rt::{self as rt, Runtime, RuntimeRef, ThreadLocal};

// This example shows how to use the restart_supervisor! macro. Writing a
// supervisor can be a bit repetitive and the macro helps speed up the creation
// of a supervisor.
//
// Run using:
// $ cargo run --example 6_restart_supervisor
fn main() -> Result<(), rt::Error> {
    // Enable logging.
    std_logger::Config::logfmt().init();

    let mut runtime = Runtime::setup().build()?;

    // Spawning here is the same as in the other examples, we're only interested
    // here in using our PrintSupervisor as supervisor.
    let sync_actor = actor_fn(sync_print_actor);
    let arg = "Hello world!".to_owned();
    let supervisor = PrintSupervisor::new(arg.clone());
    let options = SyncActorOptions::default();
    runtime.spawn_sync_actor(supervisor, sync_actor, arg, options)?;

    runtime.run_on_workers(|mut runtime_ref: RuntimeRef| -> Result<(), rt::Error> {
        let print_actor = actor_fn(print_actor);
        let options = ActorOptions::default();
        let arg = "Hello world!".to_owned();
        let supervisor = PrintSupervisor::new(arg.clone());
        runtime_ref.spawn_local(supervisor, print_actor, arg, options);
        Ok(())
    })?;

    runtime.start()
}

// Create a restart supervisor for the print_actor.
restart_supervisor!(
    PrintSupervisor, // Name of the supervisor.
    String,          // Argument for the actor.
    2,               // Maximum number of restarts.
);

/// A very bad printing actor.
async fn print_actor(_: actor::Context<(), ThreadLocal>, msg: String) -> Result<(), String> {
    Err(format!("can't print message '{msg}'"))
}

/// A very bad synchronous printing actor.
fn sync_print_actor<RT>(_: sync::Context<String, RT>, msg: String) -> Result<(), String> {
    Err(format!("can't print message synchronously '{msg}'"))
}
