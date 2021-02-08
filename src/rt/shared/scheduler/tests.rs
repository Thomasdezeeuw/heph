//! Tests for the shared scheduler.

use std::future::{pending, Pending};
use std::mem::size_of;
use std::sync::{Arc, Mutex};

use crate::actor::{self, context, NewActor};
use crate::rt::process::{ProcessId, ProcessResult};
use crate::rt::shared::scheduler::{Priority, ProcessData, Scheduler};
use crate::supervisor::NoSupervisor;
use crate::test::{self, init_actor_with_inbox, AssertUnmoved};

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

async fn simple_actor(_: actor::Context<!, context::ThreadSafe>) -> Result<(), !> {
    Ok(())
}

#[test]
fn adding_actor() {
    let scheduler = Scheduler::new();

    // Shouldn't run any process yet, since none are added.
    assert!(!scheduler.has_process());
    assert!(!scheduler.has_ready_process());
    assert_eq!(scheduler.try_steal(), None);

    // Add an actor to the scheduler.
    let actor_entry = scheduler.add_actor();
    let pid = actor_entry.pid();
    let new_actor = simple_actor as fn(_) -> _;
    let (actor, inbox, _) = init_actor_with_inbox(new_actor, ()).unwrap();
    actor_entry.add(
        Priority::NORMAL,
        NoSupervisor,
        new_actor,
        actor,
        inbox,
        false,
    );

    // Newly added processes aren't ready by default.
    assert!(scheduler.has_process());
    assert!(!scheduler.has_ready_process());
    assert_eq!(scheduler.try_steal(), None);

    // After scheduling the process should be ready to run.
    scheduler.mark_ready(pid);
    assert!(scheduler.has_process());
    assert!(scheduler.has_ready_process());
    let process = scheduler.try_steal().unwrap();
    assert_eq!(process.as_ref().id(), pid);

    // After the process is run, and returned `ProcessResult::Complete`, it
    // should be removed.
    assert!(!scheduler.has_process());
    assert!(!scheduler.has_ready_process());
    assert_eq!(scheduler.try_steal(), None);
    assert!(!scheduler.has_process());
    assert!(!scheduler.has_ready_process());

    // Adding the process back means its not ready.
    scheduler.add_process(process);
    assert!(scheduler.has_process());
    assert!(!scheduler.has_ready_process());
    assert_eq!(scheduler.try_steal(), None);

    // Marking the same process as ready again.
    scheduler.mark_ready(pid);
    assert!(scheduler.has_process());
    assert!(scheduler.has_ready_process());
    let process = scheduler.try_steal().unwrap();
    assert_eq!(process.as_ref().id(), pid);
}

#[test]
fn marking_unknown_pid_as_ready() {
    let scheduler = Scheduler::new();

    assert!(!scheduler.has_process());
    assert!(!scheduler.has_ready_process());
    assert_eq!(scheduler.try_steal(), None);

    // Scheduling an unknown process should do nothing.
    scheduler.mark_ready(ProcessId(0));
    assert!(!scheduler.has_process());
    assert!(!scheduler.has_ready_process());
    assert_eq!(scheduler.try_steal(), None);
}

#[test]
fn scheduler_run_order() {
    let scheduler = Scheduler::new();
    let mut runtime_ref = test::runtime();

    // The order in which the processes have been run.
    let run_order = Arc::new(Mutex::new(Vec::new()));

    async fn order_actor(
        _: actor::Context<!, context::ThreadSafe>,
        id: usize,
        order: Arc<Mutex<Vec<usize>>>,
    ) -> Result<(), !> {
        order.lock().unwrap().push(id);
        Ok(())
    }

    // Add our processes.
    let new_actor = order_actor as fn(_, _, _) -> _;
    let priorities = [Priority::LOW, Priority::NORMAL, Priority::HIGH];
    let mut pids = vec![];
    for (id, priority) in priorities.iter().enumerate() {
        let actor_entry = scheduler.add_actor();
        pids.push(actor_entry.pid());
        let (actor, inbox, _) = init_actor_with_inbox(new_actor, (id, run_order.clone())).unwrap();
        actor_entry.add(*priority, NoSupervisor, new_actor, actor, inbox, true);
    }

    assert!(scheduler.has_process());
    assert!(scheduler.has_ready_process());

    // Run all processes, should be in order of priority (since there runtimes
    // are equal).
    for _ in 0..3 {
        let mut process = scheduler.try_steal().unwrap();
        assert_eq!(
            process.as_mut().run(&mut runtime_ref),
            ProcessResult::Complete
        );
    }
    assert!(!scheduler.has_process());
    assert_eq!(*run_order.lock().unwrap(), vec![2usize, 1, 0]);
}

struct TestAssertUnmovedNewActor;

impl NewActor for TestAssertUnmovedNewActor {
    type Message = ();
    type Argument = ();
    type Actor = AssertUnmoved<Pending<Result<(), !>>>;
    type Error = !;
    type Context = context::ThreadSafe;

    fn new(
        &mut self,
        _: actor::Context<Self::Message, Self::Context>,
        _: Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        Ok(AssertUnmoved::new(pending()))
    }
}

#[test]
fn assert_process_unmoved() {
    let scheduler = Scheduler::new();
    let mut runtime_ref = test::runtime();

    let (actor, inbox, _) = init_actor_with_inbox(TestAssertUnmovedNewActor, ()).unwrap();

    let actor_entry = scheduler.add_actor();
    let pid = actor_entry.pid();
    actor_entry.add(
        Priority::NORMAL,
        NoSupervisor,
        TestAssertUnmovedNewActor,
        actor,
        inbox,
        true,
    );

    // Run the process multiple times, ensure it's not moved in the
    // process.
    let mut process = scheduler.try_steal().unwrap();
    assert_eq!(
        process.as_mut().run(&mut runtime_ref),
        ProcessResult::Pending
    );
    scheduler.add_process(process);

    scheduler.mark_ready(pid);
    let mut process = scheduler.try_steal().unwrap();
    assert_eq!(
        process.as_mut().run(&mut runtime_ref),
        ProcessResult::Pending
    );
    scheduler.add_process(process);

    scheduler.mark_ready(pid);
    let mut process = scheduler.try_steal().unwrap();
    assert_eq!(
        process.as_mut().run(&mut runtime_ref),
        ProcessResult::Pending
    );
}
