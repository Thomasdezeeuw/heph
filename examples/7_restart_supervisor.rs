use std::thread::sleep;
use std::time::Duration;

use heph::actor::sync::SyncContext;
use heph::rt::{self, ActorOptions, Runtime, RuntimeRef, SyncActorOptions};
use heph::{actor, restart_supervisor};

fn main() -> Result<(), rt::Error> {
    heph::log::init();
    let mut runtime = Runtime::new()?;

    let sync_actor = sync_print_actor as fn(_, _) -> _;
    let arg = "Hello world!".to_owned();
    let supervisor = PrintSupervisor::new(arg.clone());
    let options = SyncActorOptions::default();
    runtime.spawn_sync_actor(supervisor, sync_actor, arg, options)?;

    // NOTE: this is only here to make the test pass.
    sleep(Duration::from_millis(100));

    runtime
        .with_setup(|mut runtime_ref: RuntimeRef| {
            let print_actor = print_actor as fn(_, _) -> _;
            let options = ActorOptions::default().mark_ready();
            let arg = "Hello world!".to_owned();
            let supervisor = PrintSupervisor::new(arg.clone());
            runtime_ref.spawn_local(supervisor, print_actor, arg, options);
            Ok(())
        })
        .start()
}

// Create a restart supervisor for the [`print_actor`].
restart_supervisor!(
    PrintSupervisor,
    "print actor",
    String,
    default,
    default,
    ": actor message '{}'",
    args
);

/// A very bad printing actor.
async fn print_actor(_ctx: actor::Context<()>, msg: String) -> Result<(), String> {
    Err(format!("can't print message '{}'", msg))
}

/// A very bad synchronous printing actor.
fn sync_print_actor(_ctx: SyncContext<String>, msg: String) -> Result<(), String> {
    Err(format!("can't print message synchronously '{}'", msg))
}
