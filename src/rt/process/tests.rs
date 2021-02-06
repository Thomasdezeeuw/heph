//! Tests for the process module.

use std::pin::Pin;
use std::sync::atomic::{self, AtomicBool};
use std::sync::Arc;

use futures_util::future::{pending, Pending};
use mio::Token;

use crate::actor::{self, context, Actor, NewActor};
use crate::rt::process::{ActorProcess, Process, ProcessId, ProcessResult};
use crate::supervisor::{NoSupervisor, Supervisor, SupervisorStrategy};
use crate::test::{self, init_local_actor_with_inbox, AssertUnmoved, TEST_PID};

#[test]
fn pid() {
    assert_eq!(ProcessId(0), ProcessId(0));
    assert_eq!(ProcessId(100), ProcessId(100));

    assert!(ProcessId(0) < ProcessId(100));

    assert_eq!(ProcessId(0).to_string(), "0");
    assert_eq!(ProcessId(100).to_string(), "100");
    assert_eq!(ProcessId(8000).to_string(), "8000");
}

#[test]
fn pid_and_evented_id() {
    let pid = ProcessId(0);
    let id: Token = pid.into();
    assert_eq!(id, Token(0));

    let id = Token(0);
    let pid: ProcessId = id.into();
    assert_eq!(pid, ProcessId(0));
}

async fn ok_actor(mut ctx: actor::Context<()>) -> Result<(), !> {
    assert_eq!(ctx.receive_next().await, Ok(()));
    Ok(())
}

#[test]
fn actor_process() {
    // Create our actor.
    let new_actor = ok_actor as fn(_) -> _;
    let (actor, inbox, actor_ref) = init_local_actor_with_inbox(new_actor, ()).unwrap();

    // Create our process.
    let process = ActorProcess::new(NoSupervisor, new_actor, actor, inbox);
    let mut process = Box::pin(process);

    // Actor should return `Poll::Pending` in the first call, since no message
    // is available.
    let mut runtime_ref = test::runtime();
    let res = process.as_mut().run(&mut runtime_ref, TEST_PID);
    assert_eq!(res, ProcessResult::Pending);

    // Send the message and the actor should return Ok.
    actor_ref.try_send(()).unwrap();
    let res = process.as_mut().run(&mut runtime_ref, TEST_PID);
    assert_eq!(res, ProcessResult::Complete);
}

async fn error_actor(mut ctx: actor::Context<()>, fail: bool) -> Result<(), ()> {
    if fail {
        Err(())
    } else {
        assert_eq!(ctx.receive_next().await, Ok(()));
        Ok(())
    }
}

#[test]
fn erroneous_actor_process() {
    // Create our actor.
    let new_actor = error_actor as fn(_, _) -> _;
    let (actor, inbox, _) = init_local_actor_with_inbox(new_actor, true).unwrap();

    // Create our process.
    let process = ActorProcess::new(|_| SupervisorStrategy::Stop, new_actor, actor, inbox);
    let mut process = Box::pin(process);

    // Actor should return Err.
    let mut runtime_ref = test::runtime();
    let res = process.as_mut().run(&mut runtime_ref, TEST_PID);
    assert_eq!(res, ProcessResult::Complete);
}

#[test]
fn restarting_erroneous_actor_process() {
    // Create our actor.
    let new_actor = error_actor as fn(_, _) -> _;
    let (actor, inbox, actor_ref) = init_local_actor_with_inbox(new_actor, true).unwrap();

    struct TestSupervisor(Arc<AtomicBool>);

    impl<NA> Supervisor<NA> for TestSupervisor
    where
        NA: NewActor<Argument = bool>,
    {
        fn decide(&mut self, _: <NA::Actor as Actor>::Error) -> SupervisorStrategy<NA::Argument> {
            self.0.store(true, atomic::Ordering::SeqCst);
            SupervisorStrategy::Restart(false)
        }

        fn decide_on_restart_error(&mut self, _: NA::Error) -> SupervisorStrategy<NA::Argument> {
            self.0.store(true, atomic::Ordering::SeqCst);
            SupervisorStrategy::Restart(false)
        }

        fn second_restart_error(&mut self, _: NA::Error) {
            unreachable!("test call to second_restart_error in ActorProcess");
        }
    }

    let supervisor_called = Arc::new(AtomicBool::new(false));
    let supervisor = TestSupervisor(Arc::clone(&supervisor_called));

    // Create our process.
    let process = ActorProcess::new(supervisor, new_actor, actor, inbox);
    let mut process: Pin<Box<dyn Process>> = Box::pin(process);

    // In the first call to run the actor should return an error. Then it should
    // be restarted. The restarted actor waits for a message, returning
    // `Poll::Pending`.
    let mut runtime_ref = test::runtime();
    let res = process.as_mut().run(&mut runtime_ref, ProcessId(0));
    assert_eq!(res, ProcessResult::Pending);
    // Supervisor must be called and the actor restarted.
    assert!(supervisor_called.load(atomic::Ordering::SeqCst));

    // Now we send a message to the restarted actor, which should return `Ok`.
    actor_ref.try_send(()).unwrap();
    let res = process.as_mut().run(&mut runtime_ref, ProcessId(0));
    assert_eq!(res, ProcessResult::Complete);
}

struct TestAssertUnmovedNewActor;

impl NewActor for TestAssertUnmovedNewActor {
    type Message = ();
    type Argument = ();
    type Actor = AssertUnmoved<Pending<Result<(), !>>>;
    type Error = !;
    type Context = context::ThreadLocal;

    fn new(
        &mut self,
        _: actor::Context<Self::Message>,
        _: Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        Ok(AssertUnmoved::new(pending()))
    }
}

#[test]
fn actor_process_assert_actor_unmoved() {
    let (actor, inbox, _) = init_local_actor_with_inbox(TestAssertUnmovedNewActor, ()).unwrap();
    let process = ActorProcess::new(NoSupervisor, TestAssertUnmovedNewActor, actor, inbox);
    let mut process: Pin<Box<dyn Process>> = Box::pin(process);

    // All we do is run it a couple of times, it should panic if the actor is
    // moved.
    let mut runtime_ref = test::runtime();
    let res = process.as_mut().run(&mut runtime_ref, ProcessId(0));
    assert_eq!(res, ProcessResult::Pending);
    let res = process.as_mut().run(&mut runtime_ref, ProcessId(0));
    assert_eq!(res, ProcessResult::Pending);
    let res = process.as_mut().run(&mut runtime_ref, ProcessId(0));
    assert_eq!(res, ProcessResult::Pending);
}
