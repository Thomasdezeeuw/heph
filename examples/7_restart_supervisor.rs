#![feature(never_type)]

use std::thread::sleep;
use std::time::Duration;

use heph::actor::{self, SyncContext};
use heph::restart_supervisor;
use heph::rt::{self, ActorOptions, Runtime, RuntimeRef, SyncActorOptions};

fn main() -> Result<(), rt::Error> {
    std_logger::init();
    let mut runtime = Runtime::setup().build()?;

    let sync_actor = sync_print_actor as fn(_, _) -> _;
    let arg = "Hello world!".to_owned();
    let supervisor = PrintSupervisor::new(arg.clone());
    let options = SyncActorOptions::default();
    runtime.spawn_sync_actor(supervisor, sync_actor, arg, options)?;

    // NOTE: this is only here to make the test pass.
    sleep(Duration::from_millis(100));

    runtime.run_on_workers(|mut runtime_ref: RuntimeRef| -> Result<(), !> {
        let print_actor = print_actor as fn(_, _) -> _;
        let options = ActorOptions::default();
        let arg = "Hello world!".to_owned();
        let supervisor = PrintSupervisor::new(arg.clone());
        runtime_ref.spawn_local(supervisor, print_actor, arg, options);
        Ok(())
    })?;

    runtime.start()
}

// Create a restart supervisor for the [`print_actor`].
restart_supervisor!(
    PrintSupervisor,
    "print actor",
    String,
    5,
    Duration::from_secs(5),
    ": actor message '{}'",
    args
);

/// A very bad printing actor.
async fn print_actor(_: actor::Context<()>, msg: String) -> Result<(), String> {
    Err(format!("can't print message '{}'", msg))
}

/// A very bad synchronous printing actor.
fn sync_print_actor(_: SyncContext<String>, msg: String) -> Result<(), String> {
    Err(format!("can't print message synchronously '{}'", msg))
}
