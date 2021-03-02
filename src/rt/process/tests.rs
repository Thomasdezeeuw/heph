//! Tests for the process module.

use std::cmp::Ordering;
use std::future::{pending, Pending};
use std::mem::size_of;
use std::pin::Pin;
use std::sync::atomic::{self, AtomicBool};
use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;

use mio::Token;

use crate::actor::{self, Actor, NewActor};
use crate::rt::process::{ActorProcess, Process, ProcessData, ProcessId, ProcessResult};
use crate::rt::{RuntimeRef, ThreadLocal};
use crate::spawn::options::Priority;
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

fn assert_size<T>(expected: usize) {
    assert_eq!(size_of::<T>(), expected);
}

#[test]
fn size_assertions() {
    assert_size::<ProcessId>(8);
    assert_size::<Priority>(1);
    assert_size::<ProcessData<Box<dyn Process>>>(32);
}

#[derive(Debug)]
struct NopTestProcess;

impl Process for NopTestProcess {
    fn name(&self) -> &'static str {
        "NopTestProcess"
    }

    fn run(self: Pin<&mut Self>, _: &mut RuntimeRef, _: ProcessId) -> ProcessResult {
        unimplemented!();
    }
}

#[test]
#[allow(clippy::eq_op)] // Need to compare `ProcessData` to itself.
fn process_data_equality() {
    let process1 = ProcessData::new(Priority::LOW, Box::pin(NopTestProcess));
    let process2 = ProcessData::new(Priority::NORMAL, Box::pin(NopTestProcess));
    let process3 = ProcessData::new(Priority::HIGH, Box::pin(NopTestProcess));

    // Equality is only based on id alone.
    assert_eq!(process1, process1);
    assert_ne!(process1, process2);
    assert_ne!(process1, process3);

    assert_ne!(process2, process1);
    assert_eq!(process2, process2);
    assert_ne!(process2, process3);

    assert_ne!(process3, process1);
    assert_ne!(process3, process2);
    assert_eq!(process3, process3);
}

#[test]
fn process_data_ordering() {
    let mut process1 = ProcessData::new(Priority::HIGH, Box::pin(NopTestProcess));
    let mut process2 = ProcessData::new(Priority::NORMAL, Box::pin(NopTestProcess));
    let mut process3 = ProcessData::new(Priority::LOW, Box::pin(NopTestProcess));

    // Ordering only on runtime and priority.
    assert_eq!(process1.cmp(&process1), Ordering::Equal);
    assert_eq!(process1.cmp(&process2), Ordering::Greater);
    assert_eq!(process1.cmp(&process3), Ordering::Greater);

    assert_eq!(process2.cmp(&process1), Ordering::Less);
    assert_eq!(process2.cmp(&process2), Ordering::Equal);
    assert_eq!(process2.cmp(&process3), Ordering::Greater);

    assert_eq!(process3.cmp(&process1), Ordering::Less);
    assert_eq!(process3.cmp(&process2), Ordering::Less);
    assert_eq!(process3.cmp(&process3), Ordering::Equal);

    let duration = Duration::from_millis(0);
    process1.fair_runtime = duration;
    process2.fair_runtime = duration;
    process3.fair_runtime = duration;

    // If all the "fair runtimes" are equal we only compare based on the
    // priority.
    assert_eq!(process1.cmp(&process1), Ordering::Equal);
    assert_eq!(process1.cmp(&process2), Ordering::Greater);
    assert_eq!(process1.cmp(&process3), Ordering::Greater);

    assert_eq!(process2.cmp(&process1), Ordering::Less);
    assert_eq!(process2.cmp(&process2), Ordering::Equal);
    assert_eq!(process2.cmp(&process3), Ordering::Greater);

    assert_eq!(process3.cmp(&process1), Ordering::Less);
    assert_eq!(process3.cmp(&process2), Ordering::Less);
    assert_eq!(process3.cmp(&process3), Ordering::Equal);
}

#[derive(Debug)]
struct SleepyProcess(Duration);

impl Process for SleepyProcess {
    fn name(&self) -> &'static str {
        "SleepyProcess"
    }

    fn run(self: Pin<&mut Self>, _: &mut RuntimeRef, _: ProcessId) -> ProcessResult {
        sleep(self.0);
        ProcessResult::Pending
    }
}

#[test]
fn process_data_runtime_increase() {
    const SLEEP_TIME: Duration = Duration::from_millis(10);

    let mut process = Box::pin(ProcessData::new(
        Priority::HIGH,
        Box::pin(SleepyProcess(SLEEP_TIME)),
    ));
    process.fair_runtime = Duration::from_millis(10);

    // Runtime must increase after running.
    let mut runtime_ref = test::runtime();
    let res = process.as_mut().run(&mut runtime_ref);
    assert_eq!(res, ProcessResult::Pending);
    assert!(process.fair_runtime >= SLEEP_TIME);
}

async fn ok_actor(mut ctx: actor::Context<(), ThreadLocal>) {
    assert_eq!(ctx.receive_next().await, Ok(()));
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

async fn error_actor(mut ctx: actor::Context<(), ThreadLocal>, fail: bool) -> Result<(), ()> {
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
    type RuntimeAccess = ThreadLocal;

    fn new(
        &mut self,
        _: actor::Context<Self::Message, ThreadLocal>,
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
