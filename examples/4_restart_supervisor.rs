use std::time::Duration;

use heph::actor::{self, actor_fn};
use heph::future::ActorFuture;
use heph::supervisor::restart_supervisor;
use heph::sync::{self, SyncActorRunner};

mod runtime;

fn main() {
    // Enable logging.
    std_logger::Config::logfmt().init();

    // See example 1 Hell World for more information on starting and running
    // asynchronous actors.
    let arg = "Hello, World!";
    let supervisor = PrintSupervisor::new(arg);
    let (future, _) = ActorFuture::new(supervisor, actor_fn(print_actor), arg).unwrap();

    // We run our `future` on our runtime.
    runtime::block_on(future);

    // See example 3 Sync Actors for more information on starting and running
    // synchronous actors.
    let supervisor = PrintSupervisor::new(arg);
    let (sync_actor, _) = SyncActorRunner::new(supervisor, actor_fn(sync_print_actor));
    sync_actor.run(arg);
}

// Create a restart supervisor for the actors below.
restart_supervisor!(
    PrintSupervisor,         // Name of the supervisor type.
    &'static str,            // Argument for the actor.
    5,                       // Maximum number of restarts.
    Duration::from_secs(30), // Time to reset the max. reset counter.
);

/// A very bad printing actor.
async fn print_actor(_: actor::Context<()>, msg: &'static str) -> Result<(), String> {
    Err(format!("can't print message '{msg}'"))
}

/// A very bad synchronous printing actor.
fn sync_print_actor(_: sync::Context<()>, msg: &'static str) -> Result<(), String> {
    Err(format!("can't print message synchronously '{msg}'"))
}
