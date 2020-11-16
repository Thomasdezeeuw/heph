//! The setup function should be dropped after the runtime is started.
//!
//! In this test the actor reference, to the sync actor, should be dropped allow
//! the sync actor to stop and not prevent the test from returning.

use heph::actor::sync::SyncContext;
use heph::rt::{Runtime, SyncActorOptions};
use heph::supervisor::NoSupervisor;

#[test]
fn issue_294() {
    let mut runtime = Runtime::new().unwrap();

    let actor = actor as fn(_) -> _;
    let options = SyncActorOptions::default();
    let actor_ref = runtime
        .spawn_sync_actor(NoSupervisor, actor, (), options)
        .unwrap();

    runtime
        .with_setup::<_, !>(move |_| {
            actor_ref.try_send(()).unwrap();
            Ok(())
        })
        .start()
        .unwrap();
}

fn actor(mut ctx: SyncContext<()>) -> Result<(), !> {
    while let Ok(_msg) = ctx.receive_next() {}
    Ok(())
}
