//! Tests for the shared scheduler.

use std::future::{pending, Pending};
use std::mem::size_of;
use std::sync::{Arc, Mutex};

use heph::actor::{self, NewActor};
use heph::supervisor::NoSupervisor;

use crate::process::{ProcessId, ProcessResult};
use crate::shared::scheduler::{Priority, ProcessData, Scheduler};
use crate::test::{self, init_actor_future, AssertUnmoved};
use crate::ThreadSafe;

fn assert_size<T>(expected: usize) {
    assert_eq!(size_of::<T>(), expected);
}

#[test]
fn size_assertions() {
    assert_size::<ProcessData>(40);
}

#[test]
fn is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<Scheduler>();
}

async fn simple_actor(_: actor::Context<!, ThreadSafe>) {}

#[test]
fn adding_actor() {
    let scheduler = Scheduler::new();

    // Shouldn't run any process yet, since none are added.
    assert!(!scheduler.has_process());
    assert!(!scheduler.has_ready_process());
    assert_eq!(scheduler.remove(), None);

    // Add an actor to the scheduler.
    let actor_entry = scheduler.add_actor();
    let pid = actor_entry.pid();
    let new_actor = simple_actor as fn(_) -> _;
    let (future, _) = init_actor_future(NoSupervisor, new_actor, ()).unwrap();
    actor_entry.add(future, Priority::NORMAL);

    // Newly added processes are ready by default.
    assert!(scheduler.has_process());
    assert!(scheduler.has_ready_process());
    let process = scheduler.remove().unwrap();
    scheduler.add_process(process);

    // After scheduling the process should be ready to run.
    scheduler.mark_ready(pid);
    assert!(scheduler.has_process());
    assert!(scheduler.has_ready_process());
    let process = scheduler.remove().unwrap();
    assert_eq!(process.as_ref().id(), pid);

    // After the process is run, and returned `ProcessResult::Complete`, it
    // should be removed.
    assert!(!scheduler.has_process());
    assert!(!scheduler.has_ready_process());
    assert_eq!(scheduler.remove(), None);
    assert!(!scheduler.has_process());
    assert!(!scheduler.has_ready_process());

    // Adding the process back means its not ready.
    scheduler.add_process(process);
    assert!(scheduler.has_process());
    assert!(!scheduler.has_ready_process());
    assert_eq!(scheduler.remove(), None);

    // Marking the same process as ready again.
    scheduler.mark_ready(pid);
    assert!(scheduler.has_process());
    assert!(scheduler.has_ready_process());
    let process = scheduler.remove().unwrap();
    assert_eq!(process.as_ref().id(), pid);
}

#[test]
fn marking_unknown_pid_as_ready() {
    let scheduler = Scheduler::new();

    assert!(!scheduler.has_process());
    assert!(!scheduler.has_ready_process());
    assert_eq!(scheduler.remove(), None);

    // Scheduling an unknown process should do nothing.
    scheduler.mark_ready(ProcessId(0));
    assert!(!scheduler.has_process());
    assert!(!scheduler.has_ready_process());
    assert_eq!(scheduler.remove(), None);
}

#[test]
fn scheduler_run_order() {
    async fn order_actor(
        _: actor::Context<!, ThreadSafe>,
        id: usize,
        order: Arc<Mutex<Vec<usize>>>,
    ) {
        order.lock().unwrap().push(id);
    }

    let scheduler = Scheduler::new();
    let mut runtime_ref = test::runtime();

    // The order in which the processes have been run.
    let run_order = Arc::new(Mutex::new(Vec::new()));

    // Add our processes.
    let new_actor = order_actor as fn(_, _, _) -> _;
    let priorities = [Priority::LOW, Priority::NORMAL, Priority::HIGH];
    let mut pids = vec![];
    for (id, priority) in priorities.iter().enumerate() {
        let actor_entry = scheduler.add_actor();
        pids.push(actor_entry.pid());
        let (future, _) =
            init_actor_future(NoSupervisor, new_actor, (id, run_order.clone())).unwrap();
        actor_entry.add(future, *priority);
    }

    assert!(scheduler.has_process());
    assert!(scheduler.has_ready_process());

    // Run all processes, should be in order of priority (since there runtimes
    // are equal).
    for _ in 0..3 {
        let mut process = scheduler.remove().unwrap();
        assert_eq!(
            process.as_mut().run(&mut runtime_ref),
            ProcessResult::Complete
        );
    }
    assert!(!scheduler.has_process());
    assert_eq!(*run_order.lock().unwrap(), vec![2_usize, 1, 0]);
}

struct TestAssertUnmovedNewActor;

impl NewActor for TestAssertUnmovedNewActor {
    type Message = ();
    type Argument = ();
    type Actor = AssertUnmoved<Pending<Result<(), !>>>;
    type Error = !;
    type RuntimeAccess = ThreadSafe;

    fn new(
        &mut self,
        _: actor::Context<Self::Message, Self::RuntimeAccess>,
        _: Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        Ok(AssertUnmoved::new(pending()))
    }
}

#[test]
fn assert_actor_process_unmoved() {
    let scheduler = Scheduler::new();
    let mut runtime_ref = test::runtime();

    let actor_entry = scheduler.add_actor();
    let pid = actor_entry.pid();
    let (future, _) = init_actor_future(NoSupervisor, TestAssertUnmovedNewActor, ()).unwrap();
    actor_entry.add(future, Priority::NORMAL);

    // Run the process multiple times, ensure it's not moved in the
    // process.
    let mut process = scheduler.remove().unwrap();
    assert_eq!(
        process.as_mut().run(&mut runtime_ref),
        ProcessResult::Pending
    );
    scheduler.add_process(process);

    scheduler.mark_ready(pid);
    let mut process = scheduler.remove().unwrap();
    assert_eq!(
        process.as_mut().run(&mut runtime_ref),
        ProcessResult::Pending
    );
    scheduler.add_process(process);

    scheduler.mark_ready(pid);
    let mut process = scheduler.remove().unwrap();
    assert_eq!(
        process.as_mut().run(&mut runtime_ref),
        ProcessResult::Pending
    );
}

#[test]
fn assert_future_process_unmoved() {
    let scheduler = Scheduler::new();
    let mut runtime_ref = test::runtime();

    let future = AssertUnmoved::new(pending());
    scheduler.add_future(future, Priority::NORMAL);

    // Run the process multiple times, ensure it's not moved in the
    // process.
    let mut process = scheduler.remove().unwrap();
    let pid = process.as_ref().id();
    assert_eq!(
        process.as_mut().run(&mut runtime_ref),
        ProcessResult::Pending
    );
    scheduler.add_process(process);

    scheduler.mark_ready(pid);
    let mut process = scheduler.remove().unwrap();
    assert_eq!(
        process.as_mut().run(&mut runtime_ref),
        ProcessResult::Pending
    );
    scheduler.add_process(process);

    scheduler.mark_ready(pid);
    let mut process = scheduler.remove().unwrap();
    assert_eq!(
        process.as_mut().run(&mut runtime_ref),
        ProcessResult::Pending
    );
}
