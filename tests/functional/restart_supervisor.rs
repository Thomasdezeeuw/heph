//! Tests for the `restart_supervisor!` macro.

// NOTE: I'm not great with macros, so this is likely overkill. But better safe
// then sorry.

// NOTE: log format is testing in example 7 in tests/examples.rs.

use std::pin::Pin;
use std::task::{self, Poll};
use std::thread::sleep;
use std::time::Duration;

use heph::{actor, restart_supervisor, Actor, NewActor, Supervisor, SupervisorStrategy};

// NOTE: keep in sync with the documentation.
const DEFAULT_MAX_RESTARTS: usize = 5;
const DEFAULT_MAX_DURATION: Duration = Duration::from_secs(5);

#[test]
fn new_actor_unit_argument() {
    restart_supervisor!(Supervisor);
    // Should be able to create it without arguments passed to `new`.
    let _supervisor = Supervisor::new();
    assert_eq!(Supervisor::MAX_RESTARTS, DEFAULT_MAX_RESTARTS);
    assert_eq!(Supervisor::MAX_DURATION, DEFAULT_MAX_DURATION);
}

#[test]
fn new_actor_unit_argument_explicit() {
    restart_supervisor!(Supervisor, ());
    // Should be able to create it without arguments passed to `new`.
    let _supervisor = Supervisor::new();
    assert_eq!(Supervisor::MAX_RESTARTS, DEFAULT_MAX_RESTARTS);
    assert_eq!(Supervisor::MAX_DURATION, DEFAULT_MAX_DURATION);
}

#[test]
fn new_actor_single_argument() {
    restart_supervisor!(Supervisor, String);
    // Should be able to directly pass argument.
    let _supervisor = Supervisor::new("Hello World".to_owned());
    assert_eq!(Supervisor::MAX_RESTARTS, DEFAULT_MAX_RESTARTS);
    assert_eq!(Supervisor::MAX_DURATION, DEFAULT_MAX_DURATION);
}

#[test]
fn new_actor_tuple_argument() {
    restart_supervisor!(Supervisor, (String, usize));
    // Should be able to directly pass argument.
    let _supervisor = Supervisor::new("Hello World".to_owned(), 123);
    assert_eq!(Supervisor::MAX_RESTARTS, DEFAULT_MAX_RESTARTS);
    assert_eq!(Supervisor::MAX_DURATION, DEFAULT_MAX_DURATION);
}

#[test]
fn no_log_unit_argument() {
    restart_supervisor!(Supervisor, (), 2, Duration::from_secs(10));
    let _supervisor = Supervisor::new();
    assert_eq!(Supervisor::MAX_RESTARTS, 2);
    assert_eq!(Supervisor::MAX_DURATION, Duration::from_secs(10));
}

#[test]
fn no_log_single_argument() {
    restart_supervisor!(Supervisor, usize, 2, Duration::from_secs(10));
    let _supervisor = Supervisor::new(123);
    assert_eq!(Supervisor::MAX_RESTARTS, 2);
    assert_eq!(Supervisor::MAX_DURATION, Duration::from_secs(10));
}

#[test]
fn no_log_tuple_argument() {
    restart_supervisor!(Supervisor, (u8, u16), 2, Duration::from_secs(10));
    let _supervisor = Supervisor::new(123, 456);
    assert_eq!(Supervisor::MAX_RESTARTS, 2);
    assert_eq!(Supervisor::MAX_DURATION, Duration::from_secs(10));
}

#[test]
fn all_unit_argument() {
    restart_supervisor!(Supervisor, (), 2, Duration::from_secs(10), ": log extra",);
    let _supervisor = Supervisor::new();
    assert_eq!(Supervisor::MAX_RESTARTS, 2);
    assert_eq!(Supervisor::MAX_DURATION, Duration::from_secs(10));
}

#[test]
fn all_single_argument() {
    restart_supervisor!(
        Supervisor,
        usize,
        2,
        Duration::from_secs(10),
        ": log extra: {}",
        args
    );
    let _supervisor = Supervisor::new(123);
    assert_eq!(Supervisor::MAX_RESTARTS, 2);
    assert_eq!(Supervisor::MAX_DURATION, Duration::from_secs(10));
}

#[test]
fn all_tuple_argument() {
    restart_supervisor!(
        Supervisor,
        (u8, u16),
        2,
        Duration::from_secs(10),
        ": log extra: {}, {}",
        args.0,
        args.1,
    );
    let _supervisor = Supervisor::new(123, 456);
    assert_eq!(Supervisor::MAX_RESTARTS, 2);
    assert_eq!(Supervisor::MAX_DURATION, Duration::from_secs(10));
}

#[test]
fn tuple_2() {
    restart_supervisor!(Supervisor, (String, usize));
    // Should be able to directly pass argument.
    let _supervisor = Supervisor::new("Hello World".to_owned(), 123);
}

#[test]
fn tuple_3() {
    restart_supervisor!(Supervisor, (String, usize, u8));
    // Should be able to directly pass argument.
    let _supervisor = Supervisor::new("Hello World".to_owned(), 123, 1);
}

#[test]
fn tuple_4() {
    restart_supervisor!(Supervisor, (String, usize, u8, &'static str));
    // Should be able to directly pass argument.
    let _supervisor = Supervisor::new("Hello World".to_owned(), 123, 1, "arg");
}

#[test]
fn tuple_5() {
    restart_supervisor!(Supervisor, (String, usize, u8, &'static str, u8));
    // Should be able to directly pass argument.
    let _supervisor = Supervisor::new("Hello World".to_owned(), 123, 1, "arg", 1);
}

#[test]
fn tuple_6() {
    restart_supervisor!(Supervisor, (String, usize, u8, u8, u8, u8));
    // Should be able to directly pass argument.
    let _supervisor = Supervisor::new("Hello World".to_owned(), 123, 1, 2, 3, 4);
}

#[test]
fn tuple_7() {
    restart_supervisor!(Supervisor, (String, usize, u8, u8, u8, u8, u8));
    // Need to use tuple format.
    let _supervisor = Supervisor::new(("Hello World".to_owned(), 123, 1, 2, 3, 4, 5));
}

const ERROR1: &str = "error 1";
const ERROR2: &str = "error 2";

const NEW_ACTOR: NewActorImpl = NewActorImpl;

struct NewActorImpl;

impl NewActor for NewActorImpl {
    type Message = !;
    type Argument = bool;
    type Actor = ActorImpl;
    type Error = &'static str;
    type RuntimeAccess = ();

    fn new(
        &mut self,
        _: actor::Context<Self::Message, Self::RuntimeAccess>,
        _: Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        unimplemented!()
    }
}

struct ActorImpl;

impl Actor for ActorImpl {
    type Error = &'static str;

    fn try_poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Result<(), Self::Error>> {
        unimplemented!()
    }
}

/// Helper function to call `supervisor.decide` with `NA` as `NewActor`
/// implementation.
///
/// Only here because the following doesn't work:
///
/// ```no_run
/// assert_eq!(supervisor.decide(ERROR1), SupervisorStrategy::Restart(arg));
/// ```
fn decide_for<NA, S>(
    _: &NA, // Just here for easy of use.
    supervisor: &mut S,
    err: <<NA as NewActor>::Actor as Actor>::Error,
) -> SupervisorStrategy<NA::Argument>
where
    NA: NewActor,
    S: Supervisor<NA>,
{
    supervisor.decide(err)
}

/// See [`decide_for`].
fn decide_for_restart<NA, S>(
    _: &NA, // Just here for easy of use.
    supervisor: &mut S,
    err: <NA as NewActor>::Error,
) -> SupervisorStrategy<NA::Argument>
where
    NA: NewActor,
    S: Supervisor<NA>,
{
    supervisor.decide_on_restart_error(err)
}

/// See [`decide_for`].
fn decide_for_restart_second<NA, S>(
    _: &NA, // Just here for easy of use.
    supervisor: &mut S,
    err: <NA as NewActor>::Error,
) where
    NA: NewActor,
    S: Supervisor<NA>,
{
    supervisor.second_restart_error(err)
}

#[test]
fn decide() {
    restart_supervisor!(Supervisor, bool, 1, Duration::from_secs(60));

    let arg = true;
    let mut supervisor = Supervisor::new(arg);

    assert_eq!(
        decide_for(&NEW_ACTOR, &mut supervisor, ERROR1),
        SupervisorStrategy::Restart(arg)
    );
    assert_eq!(
        decide_for(&NEW_ACTOR, &mut supervisor, ERROR1),
        SupervisorStrategy::Stop
    );
}

#[test]
fn decide_max_duration_elapsed() {
    restart_supervisor!(Supervisor, bool, 1, Duration::from_millis(100));

    let arg = true;
    let mut supervisor = Supervisor::new(arg);

    assert_eq!(
        decide_for(&NEW_ACTOR, &mut supervisor, ERROR1),
        SupervisorStrategy::Restart(arg)
    );
    assert_eq!(
        decide_for(&NEW_ACTOR, &mut supervisor, ERROR1),
        SupervisorStrategy::Stop
    );

    // After waiting for a while the actor should be allowed to restart.
    sleep(Supervisor::MAX_DURATION);
    assert_eq!(
        decide_for(&NEW_ACTOR, &mut supervisor, ERROR1),
        SupervisorStrategy::Restart(arg)
    );
    assert_eq!(
        decide_for(&NEW_ACTOR, &mut supervisor, ERROR1),
        SupervisorStrategy::Stop
    );
}

#[test]
fn decide_on_restart_error() {
    restart_supervisor!(Supervisor, bool, 1, Duration::from_secs(60));

    let arg = true;
    let mut supervisor = Supervisor::new(arg);

    assert_eq!(
        decide_for_restart(&NEW_ACTOR, &mut supervisor, ERROR2),
        SupervisorStrategy::Restart(arg)
    );
    assert_eq!(
        decide_for_restart(&NEW_ACTOR, &mut supervisor, ERROR2),
        SupervisorStrategy::Stop
    );
}

#[test]
fn decide_on_second_restart_error() {
    restart_supervisor!(Supervisor, bool, 1, Duration::from_secs(60));

    let arg = true;
    let mut supervisor = Supervisor::new(arg);
    decide_for_restart_second(&NEW_ACTOR, &mut supervisor, ERROR2);
}
