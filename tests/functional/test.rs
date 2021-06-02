//! Tests for the `test` module.

use std::future::pending;
use std::future::poll_fn;
use std::mem::size_of;
use std::pin::Pin;
use std::task::{self, Poll};
use std::time::{Duration, Instant};

use heph::actor::{self, Actor, NewActor};
use heph::rt::{self, ThreadLocal};
use heph::spawn::{ActorOptions, FutureOptions};
use heph::supervisor::NoSupervisor;
use heph::test::{
    join, join_many, size_of_actor, size_of_actor_val, spawn_future, try_spawn, try_spawn_local,
    JoinResult,
};
use heph::timer::Timer;

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

/// Actor that panics.
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

/// Sleep time of [`sleepy_actor`].
const SLEEP_TIME: Duration = Duration::from_millis(200);

/// Actor that sleeps and then returns.
async fn sleepy_actor<RT: rt::Access + Clone>(mut ctx: actor::Context<!, RT>) {
    let _ = Timer::after(&mut ctx, SLEEP_TIME).await;
}

#[test]
fn join_local_actor() {
    let actor = sleepy_actor as fn(_) -> _;
    let actor_ref = try_spawn_local(NoSupervisor, actor, (), ActorOptions::default()).unwrap();
    let start = Instant::now();
    join(&actor_ref, Duration::from_secs(1)).unwrap();
    assert_within_margin(start, SLEEP_TIME);
}

#[test]
fn join_actor() {
    let actor = sleepy_actor as fn(_) -> _;
    let actor_ref = try_spawn(NoSupervisor, actor, (), ActorOptions::default()).unwrap();
    let start = Instant::now();
    join(&actor_ref, Duration::from_secs(1)).unwrap();
    assert_within_margin(start, SLEEP_TIME);
}

const TIMEOUT: Duration = Duration::from_millis(300);

/// Actor that never returns.
async fn never_actor<RT>(_: actor::Context<!, RT>) {
    pending::<()>().await
}

#[test]
fn join_local_actor_timeout() {
    let actor = never_actor as fn(_) -> _;
    let actor_ref = try_spawn_local(NoSupervisor, actor, (), ActorOptions::default()).unwrap();
    let start = Instant::now();
    if let JoinResult::Ok = join(&actor_ref, TIMEOUT) {
        panic!("unexpected join result");
    }
    assert_within_margin(start, TIMEOUT);
}

#[test]
fn join_actor_timeout() {
    let actor = never_actor as fn(_) -> _;
    let actor_ref = try_spawn(NoSupervisor, actor, (), ActorOptions::default()).unwrap();
    let start = Instant::now();
    if let JoinResult::Ok = join(&actor_ref, TIMEOUT) {
        panic!("unexpected join result");
    }
    assert_within_margin(start, TIMEOUT);
}

#[test]
fn join_many_local_actor() {
    let actor = sleepy_actor as fn(_) -> _;
    let actor_ref1 = try_spawn_local(NoSupervisor, actor, (), ActorOptions::default()).unwrap();
    let actor_ref2 = try_spawn_local(NoSupervisor, actor, (), ActorOptions::default()).unwrap();
    let start = Instant::now();
    join_many(&[actor_ref1, actor_ref2], Duration::from_secs(1)).unwrap();
    assert_within_margin(start, SLEEP_TIME);
}

#[test]
fn join_many_actor() {
    let actor = sleepy_actor as fn(_) -> _;
    let actor_ref1 = try_spawn(NoSupervisor, actor, (), ActorOptions::default()).unwrap();
    let actor_ref2 = try_spawn(NoSupervisor, actor, (), ActorOptions::default()).unwrap();
    let start = Instant::now();
    join_many(&[actor_ref1, actor_ref2], Duration::from_secs(1)).unwrap();
    assert_within_margin(start, SLEEP_TIME);
}

#[test]
fn join_many_local_and_thread_safe() {
    let actor = sleepy_actor as fn(_) -> _;
    let actor_ref1 = try_spawn_local(NoSupervisor, actor, (), ActorOptions::default()).unwrap();
    let actor = sleepy_actor as fn(_) -> _;
    let actor_ref2 = try_spawn(NoSupervisor, actor, (), ActorOptions::default()).unwrap();
    let start = Instant::now();
    join_many(&[actor_ref1, actor_ref2], Duration::from_secs(1)).unwrap();
    assert_within_margin(start, SLEEP_TIME);
}

#[test]
fn join_many_local_and_thread_safe_timeout() {
    let actor = sleepy_actor as fn(_) -> _;
    let actor_ref1 = try_spawn_local(NoSupervisor, actor, (), ActorOptions::default()).unwrap();
    let actor = never_actor as fn(_) -> _;
    let actor_ref2 = try_spawn(NoSupervisor, actor, (), ActorOptions::default()).unwrap();
    let start = Instant::now();
    if let JoinResult::Ok = join_many(&[actor_ref1, actor_ref2], TIMEOUT) {
        panic!("unexpected join result");
    }
    assert_within_margin(start, TIMEOUT);
}

/// Assert that less then `expected` time has elapsed since `start`.
fn assert_within_margin(start: Instant, expected: Duration) {
    const MARGIN: Duration = Duration::from_millis(150);
    let elapsed = start.elapsed();
    assert!(
        elapsed <= expected + MARGIN,
        "elapsed: {:?}, expected: {:?}",
        elapsed,
        expected
    );
}
