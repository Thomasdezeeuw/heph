//! Short running synchronous threads that are finished before calling
//! [`Runtime::start`] should be collected, and not run for ever.

use std::thread::sleep;
use std::time::Duration;

use heph::actor::SyncContext;
use heph::rt::Runtime;
use heph::spawn::SyncActorOptions;
use heph::supervisor::NoSupervisor;

#[test]
fn issue_323() {
    let mut runtime = Runtime::new().unwrap();

    let actor = actor as fn(_) -> _;
    let options = SyncActorOptions::default();
    runtime
        .spawn_sync_actor(NoSupervisor, actor, (), options)
        .unwrap();

    // Let the synchronous actor complete first.
    sleep(Duration::from_secs(1));

    // This just needs to return and not hang for ever.
    runtime.start().unwrap();
}

/// Short running synchronous actor.
fn actor(_: SyncContext<()>) {}
