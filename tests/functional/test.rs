//! Tests for the `test` module.

use std::panic;
use std::pin::Pin;
use std::task::{self, Poll};

use heph::actor::{self, actor_fn, Actor, NewActor};
use heph::sync::{self, SyncActor};
use heph::test::{size_of_actor, size_of_actor_val, PanicSupervisor};
use heph::{Supervisor, SyncSupervisor};

struct Na;

impl NewActor for Na {
    type Message = !;
    type Argument = ();
    type Actor = A;
    type Error = &'static str;
    type RuntimeAccess = ();

    fn new(
        &mut self,
        _: actor::Context<Self::Message, Self::RuntimeAccess>,
        _: Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        Ok(A)
    }
}

impl SyncActor for Na {
    type Message = !;
    type Argument = ();
    type Error = &'static str;
    type RuntimeAccess = ();

    fn run(
        &self,
        _: sync::Context<Self::Message, Self::RuntimeAccess>,
        _: Self::Argument,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

struct A;

impl Actor for A {
    type Error = &'static str;
    fn try_poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}

#[test]
fn test_size_of_actor() {
    async fn actor1(_: actor::Context<!, ()>) {
        /* Nothing. */
    }

    assert_eq!(size_of_actor_val(&actor_fn(actor1)), 24);

    assert_eq!(size_of::<A>(), 0);
    assert_eq!(size_of_actor::<Na>(), 0);
    assert_eq!(size_of_actor_val(&Na), 0);
}

#[test]
#[should_panic = "error running 'A' actor: first error"]
fn panic_supervisor_decide() {
    Supervisor::<Na>::decide(&mut PanicSupervisor, "first error");
}

#[test]
#[should_panic = "error restarting 'A' actor: restart error"]
fn panic_supervisor_decide_on_restart_error() {
    Supervisor::<Na>::decide_on_restart_error(&mut PanicSupervisor, "restart error");
}

#[test]
#[should_panic = "error restarting 'A' actor a second time: second restart error"]
fn panic_supervisor_second_restart_error() {
    Supervisor::<Na>::second_restart_error(&mut PanicSupervisor, "second restart error");
}

#[test]
#[should_panic = "original panic"]
fn panic_supervisor_decide_on_panic() {
    let panic = panic::catch_unwind(|| panic!("original panic")).unwrap_err();
    Supervisor::<Na>::decide_on_panic(&mut PanicSupervisor, panic);
}

#[test]
#[should_panic = "error running sync actor: first error"]
fn panic_supervisor_sync_decide() {
    SyncSupervisor::<Na>::decide(&mut PanicSupervisor, "first error");
}

#[test]
#[should_panic = "original panic"]
fn panic_supervisor_sync_decide_on_panic() {
    let panic = panic::catch_unwind(|| panic!("original panic")).unwrap_err();
    SyncSupervisor::<Na>::decide_on_panic(&mut PanicSupervisor, panic);
}
