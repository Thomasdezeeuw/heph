//! The setup function should be dropped after the runtime is started.
//!
//! In this test the actor reference, to the sync actor, should be dropped allow
//! the sync actor to stop and not prevent the test from returning.

use heph::actor::sync::SyncContext;
use heph::supervisor::NoSupervisor;
use heph::Runtime;

#[test]
fn issue_294() {
    let mut runtime = Runtime::new().unwrap();

    let actor = actor as fn(_) -> _;
    let actor_ref = runtime.spawn_sync_actor(NoSupervisor, actor, ()).unwrap();

    runtime
        .with_setup::<_, !>(move |_| {
            actor_ref.send(()).unwrap();
            Ok(())
        })
        .start()
        .unwrap();
}

fn actor(mut ctx: SyncContext<()>) -> Result<(), !> {
    while let Ok(msg) = ctx.receive_next() {
        drop(msg);
    }
    Ok(())
}
