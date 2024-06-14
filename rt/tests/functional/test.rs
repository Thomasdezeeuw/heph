//! Tests for the `test` module.

use std::future::pending;
use std::future::poll_fn;
use std::pin::Pin;
use std::task::{self, Poll};
use std::time::{Duration, Instant};

use heph::actor::{self, actor_fn, Actor, NewActor};
use heph::actor_ref::ActorGroup;
use heph::supervisor::NoSupervisor;
use heph_rt::spawn::{ActorOptions, FutureOptions};
use heph_rt::test::{
    self, join, join_all, join_many, size_of_actor, size_of_actor_val, spawn_future, try_spawn,
    try_spawn_local, JoinResult,
};
use heph_rt::timer::Timer;
use heph_rt::{self as rt, ThreadLocal};

#[test]
fn block_on_future() {
    let result = test::block_on_future(async move { "All good" });
    assert_eq!(result, "All good");
}

#[test]
#[should_panic = "Not good"]
fn block_on_future_panic() {
    test::block_on_future(async move { panic!("Not good") });
}

#[test]
fn test_size_of_actor() {
    async fn actor1(_: actor::Context<!, ThreadLocal>) {
        /* Nothing. */
    }

    assert_eq!(size_of_actor_val(&actor_fn(actor1)), 32);

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
    let actor = actor_fn(panic_actor);
    try_spawn_local(NoSupervisor, actor, (), ActorOptions::default()).unwrap();
}

#[test]
fn catch_panics_spawned_actor() {
    let actor = actor_fn(panic_actor);
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
async fn sleepy_actor<RT: rt::Access + Clone>(ctx: actor::Context<!, RT>) {
    let _ = Timer::after(ctx.runtime_ref().clone(), SLEEP_TIME).await;
}

#[test]
fn join_local_actor() {
    let actor = actor_fn(sleepy_actor);
    let actor_ref = try_spawn_local(NoSupervisor, actor, (), ActorOptions::default()).unwrap();
    let start = Instant::now();
    join(&actor_ref, Duration::from_secs(1)).unwrap();
    assert_within_margin(start, SLEEP_TIME);
}

#[test]
fn join_actor() {
    let actor = actor_fn(sleepy_actor);
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
    let actor = actor_fn(never_actor);
    let actor_ref = try_spawn_local(NoSupervisor, actor, (), ActorOptions::default()).unwrap();
    let start = Instant::now();
    if let JoinResult::Ok = join(&actor_ref, TIMEOUT) {
        panic!("unexpected join result");
    }
    assert_within_margin(start, TIMEOUT);
}

#[test]
fn join_actor_timeout() {
    let actor = actor_fn(never_actor);
    let actor_ref = try_spawn(NoSupervisor, actor, (), ActorOptions::default()).unwrap();
    let start = Instant::now();
    if let JoinResult::Ok = join(&actor_ref, TIMEOUT) {
        panic!("unexpected join result");
    }
    assert_within_margin(start, TIMEOUT);
}

#[test]
fn join_many_local_actors() {
    let actor = actor_fn(sleepy_actor);
    let actor_ref1 = try_spawn_local(NoSupervisor, actor, (), ActorOptions::default()).unwrap();
    let actor_ref2 = try_spawn_local(NoSupervisor, actor, (), ActorOptions::default()).unwrap();
    let start = Instant::now();
    join_many(&[actor_ref1, actor_ref2], Duration::from_secs(1)).unwrap();
    assert_within_margin(start, SLEEP_TIME);
}

#[test]
fn join_many_actors() {
    let actor = actor_fn(sleepy_actor);
    let actor_ref1 = try_spawn(NoSupervisor, actor, (), ActorOptions::default()).unwrap();
    let actor_ref2 = try_spawn(NoSupervisor, actor, (), ActorOptions::default()).unwrap();
    let start = Instant::now();
    join_many(&[actor_ref1, actor_ref2], Duration::from_secs(1)).unwrap();
    assert_within_margin(start, SLEEP_TIME);
}

#[test]
fn join_many_local_and_thread_safe() {
    let actor = actor_fn(sleepy_actor);
    let actor_ref1 = try_spawn_local(NoSupervisor, actor, (), ActorOptions::default()).unwrap();
    let actor = actor_fn(sleepy_actor);
    let actor_ref2 = try_spawn(NoSupervisor, actor, (), ActorOptions::default()).unwrap();
    let start = Instant::now();
    join_many(&[actor_ref1, actor_ref2], Duration::from_secs(1)).unwrap();
    assert_within_margin(start, SLEEP_TIME);
}

#[test]
fn join_many_local_and_thread_safe_timeout() {
    let actor = actor_fn(sleepy_actor);
    let actor_ref1 = try_spawn_local(NoSupervisor, actor, (), ActorOptions::default()).unwrap();
    let actor = actor_fn(never_actor);
    let actor_ref2 = try_spawn(NoSupervisor, actor, (), ActorOptions::default()).unwrap();
    let start = Instant::now();
    if let JoinResult::Ok = join_many(&[actor_ref1, actor_ref2], TIMEOUT) {
        panic!("unexpected join result");
    }
    assert_within_margin(start, TIMEOUT);
}

#[test]
fn join_all_local_actors() {
    let actor = actor_fn(sleepy_actor);
    let actor_ref1 = try_spawn_local(NoSupervisor, actor, (), ActorOptions::default()).unwrap();
    let actor_ref2 = try_spawn_local(NoSupervisor, actor, (), ActorOptions::default()).unwrap();
    let group = ActorGroup::from_iter([actor_ref1, actor_ref2]);
    let start = Instant::now();
    join_all(&group, Duration::from_secs(1)).unwrap();
    assert_within_margin(start, SLEEP_TIME);
}

#[test]
fn join_all_actors() {
    let actor = actor_fn(sleepy_actor);
    let actor_ref1 = try_spawn(NoSupervisor, actor, (), ActorOptions::default()).unwrap();
    let actor_ref2 = try_spawn(NoSupervisor, actor, (), ActorOptions::default()).unwrap();
    let group = ActorGroup::from_iter([actor_ref1, actor_ref2]);
    let start = Instant::now();
    join_all(&group, Duration::from_secs(1)).unwrap();
    assert_within_margin(start, SLEEP_TIME);
}

#[test]
fn join_all_local_and_thread_safe() {
    let actor = actor_fn(sleepy_actor);
    let actor_ref1 = try_spawn_local(NoSupervisor, actor, (), ActorOptions::default()).unwrap();
    let actor = actor_fn(sleepy_actor);
    let actor_ref2 = try_spawn(NoSupervisor, actor, (), ActorOptions::default()).unwrap();
    let group = ActorGroup::from_iter([actor_ref1, actor_ref2]);
    let start = Instant::now();
    join_all(&group, Duration::from_secs(1)).unwrap();
    assert_within_margin(start, SLEEP_TIME);
}

#[test]
fn join_all_local_and_thread_safe_timeout() {
    let actor = actor_fn(sleepy_actor);
    let actor_ref1 = try_spawn_local(NoSupervisor, actor, (), ActorOptions::default()).unwrap();
    let actor = actor_fn(never_actor);
    let actor_ref2 = try_spawn(NoSupervisor, actor, (), ActorOptions::default()).unwrap();
    let group = ActorGroup::from_iter([actor_ref1, actor_ref2]);
    let start = Instant::now();
    if let JoinResult::Ok = join_all(&group, TIMEOUT) {
        panic!("unexpected join result");
    }
    assert_within_margin(start, TIMEOUT);
}

/// Assert that less then `expected` time has elapsed since `start`.
#[track_caller]
fn assert_within_margin(start: Instant, expected: Duration) {
    const MARGIN: Duration = Duration::from_millis(300);
    let elapsed = start.elapsed();
    assert!(
        elapsed <= expected + MARGIN,
        "elapsed: {elapsed:?}, expected: {expected:?}",
    );
}
