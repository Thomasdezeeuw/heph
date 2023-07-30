//! The setup function should be dropped after the runtime is started.
//!
//! In this test the actor reference, to the sync actor, should be dropped allow
//! the sync actor to stop and not prevent the test from returning.

use heph::actor::actor_fn;
use heph::supervisor::NoSupervisor;
use heph::sync;
use heph_rt::spawn::SyncActorOptions;
use heph_rt::Runtime;

#[test]
fn issue_294() {
    let mut runtime = Runtime::new().unwrap();

    let actor = actor_fn(actor);
    let options = SyncActorOptions::default();
    let actor_ref = runtime
        .spawn_sync_actor(NoSupervisor, actor, (), options)
        .unwrap();

    runtime
        .run_on_workers::<_, !>(move |_| {
            actor_ref.try_send(()).unwrap();
            Ok(())
        })
        .unwrap();
    runtime.start().unwrap();
}

fn actor<RT>(mut ctx: sync::Context<(), RT>) {
    while let Ok(()) = ctx.receive_next() {}
}
