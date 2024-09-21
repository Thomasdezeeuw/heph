//! Tests for the `test` module.

use std::panic;

use heph::actor::{self, actor_fn};
use heph::test::{size_of_actor, size_of_actor_val, PanicSupervisor};
use heph::{Supervisor, SyncSupervisor};

use crate::util::{EmptyActor, EmptyNewActor};

#[test]
fn test_size_of_actor() {
    async fn actor1(_: actor::Context<!, ()>) {
        /* Nothing. */
    }

    assert_eq!(size_of_actor_val(&actor_fn(actor1)), 24);

    assert_eq!(size_of::<EmptyActor>(), 0);
    assert_eq!(size_of_actor::<EmptyNewActor>(), 0);
    assert_eq!(size_of_actor_val(&EmptyNewActor), 0);
}

#[test]
#[should_panic = "error running 'EmptyActor' actor: first error"]
fn panic_supervisor_decide() {
    Supervisor::<EmptyNewActor>::decide(&mut PanicSupervisor, "first error");
}

#[test]
#[should_panic = "error restarting 'EmptyActor' actor: restart error"]
fn panic_supervisor_decide_on_restart_error() {
    Supervisor::<EmptyNewActor>::decide_on_restart_error(&mut PanicSupervisor, "restart error");
}

#[test]
#[should_panic = "error restarting 'EmptyActor' actor a second time: second restart error"]
fn panic_supervisor_second_restart_error() {
    Supervisor::<EmptyNewActor>::second_restart_error(&mut PanicSupervisor, "second restart error");
}

#[test]
#[should_panic = "original panic"]
fn panic_supervisor_decide_on_panic() {
    let panic = panic::catch_unwind(|| panic!("original panic")).unwrap_err();
    Supervisor::<EmptyNewActor>::decide_on_panic(&mut PanicSupervisor, panic);
}

#[test]
#[should_panic = "error running 'EmptyActor' actor: first error"]
fn panic_supervisor_sync_decide() {
    SyncSupervisor::<EmptyActor>::decide(&mut PanicSupervisor, "first error");
}

#[test]
#[should_panic = "original panic"]
fn panic_supervisor_sync_decide_on_panic() {
    let panic = panic::catch_unwind(|| panic!("original panic")).unwrap_err();
    SyncSupervisor::<EmptyActor>::decide_on_panic(&mut PanicSupervisor, panic);
}
