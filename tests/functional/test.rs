//! Tests for the `test` module.

use std::future::poll_fn;
use std::mem::size_of;
use std::pin::Pin;
use std::task::{self, Poll};

use heph::actor::{self, Actor, NewActor};
use heph::rt::ThreadLocal;
use heph::spawn::{ActorOptions, FutureOptions};
use heph::supervisor::NoSupervisor;
use heph::test::{size_of_actor, size_of_actor_val, spawn_future, try_spawn, try_spawn_local};

#[test]
fn test_size_of_actor() {
    async fn actor1(_: actor::Context<!, ThreadLocal>) {
        /* Nothing. */
    }

    #[allow(trivial_casts)]
    {
        assert_eq!(size_of_actor_val(&(actor1 as fn(_) -> _)), 32);
    }

    struct Na;

    impl NewActor for Na {
        type Message = !;
        type Argument = ();
        type Actor = A;
        type Error = !;
        type RuntimeAccess = ThreadLocal;

        fn new(
            &mut self,
            _: actor::Context<Self::Message, Self::RuntimeAccess>,
            _: Self::Argument,
        ) -> Result<Self::Actor, Self::Error> {
            Ok(A)
        }
    }

    struct A;

    impl Actor for A {
        type Error = !;
        fn try_poll(
            self: Pin<&mut Self>,
            _: &mut task::Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
    }

    assert_eq!(size_of::<A>(), 0);
    assert_eq!(size_of_actor::<Na>(), 0);
    assert_eq!(size_of_actor_val(&Na), 0);
}

async fn panic_actor<RT>(_: actor::Context<!, RT>) {
    panic!("panic in actor");
}

#[test]
fn catch_panics_spawned_local_actor() {
    let actor = panic_actor as fn(_) -> _;
    try_spawn_local(NoSupervisor, actor, (), ActorOptions::default()).unwrap();
}

#[test]
fn catch_panics_spawned_actor() {
    let actor = panic_actor as fn(_) -> _;
    try_spawn(NoSupervisor, actor, (), ActorOptions::default()).unwrap();
}

#[test]
fn catch_panics_spawned_future() {
    let future = poll_fn(|_| panic!("panic in spawned Future"));
    spawn_future(future, FutureOptions::default());
}
