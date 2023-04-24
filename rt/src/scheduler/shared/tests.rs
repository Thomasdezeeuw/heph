//! Tests for the shared scheduler.

use std::future::{pending, Pending};
use std::mem::size_of;
use std::sync::{Arc, Mutex};
use std::task::{self, Poll};

use heph::actor::{self, ActorFuture, NewActor};
use heph::supervisor::NoSupervisor;

use crate::process::{FutureProcess, ProcessId};
use crate::scheduler::shared::{Priority, ProcessData, Scheduler};
use crate::test::{self, nop_task_waker, AssertUnmoved};
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

    let pid = add_test_actor(&scheduler, Priority::NORMAL);

    // Newly added processes are ready by default.
    assert!(scheduler.has_process());
    assert!(scheduler.has_ready_process());
    let process = scheduler.remove().unwrap();
    scheduler.add_back_process(process);

    // After scheduling the process should be ready to run.
    scheduler.mark_ready(pid);
    assert!(scheduler.has_process());
    assert!(scheduler.has_ready_process());
    let process = scheduler.remove().unwrap();
    assert_eq!(process.as_ref().id(), pid);

    // After the process is run, and returned `Poll::Ready(()`, it should be
    // removed.
    assert!(!scheduler.has_process());
    assert!(!scheduler.has_ready_process());
    assert_eq!(scheduler.remove(), None);
    assert!(!scheduler.has_process());
    assert!(!scheduler.has_ready_process());

    // Adding the process back means its not ready.
    scheduler.add_back_process(process);
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
    let waker = nop_task_waker();
    let mut ctx = task::Context::from_waker(&waker);

    // The order in which the processes have been run.
    let run_order = Arc::new(Mutex::new(Vec::new()));

    // Add our processes.
    let new_actor = order_actor as fn(_, _, _) -> _;
    let priorities = [Priority::LOW, Priority::NORMAL, Priority::HIGH];
    let mut pids = vec![];
    for (id, priority) in priorities.iter().enumerate() {
        let pid = scheduler
            .add_new_process(*priority, |pid| {
                let rt = ThreadSafe::new(test::shared_internals());
                ActorFuture::new(NoSupervisor, new_actor, (id, run_order.clone()), rt)
                    .map(|(future, _)| (future, pid))
            })
            .unwrap();
        pids.push(pid);
    }

    assert!(scheduler.has_process());
    assert!(scheduler.has_ready_process());

    // Run all processes, should be in order of priority (since there runtimes
    // are equal).
    for _ in 0..3 {
        let mut process = scheduler.remove().unwrap();
        assert_eq!(process.as_mut().run(&mut ctx), Poll::Ready(()));
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
    let waker = nop_task_waker();
    let mut ctx = task::Context::from_waker(&waker);

    let pid = scheduler
        .add_new_process(Priority::NORMAL, |pid| {
            let rt = ThreadSafe::new(test::shared_internals());
            ActorFuture::new(NoSupervisor, TestAssertUnmovedNewActor, (), rt)
                .map(|(future, _)| (future, pid))
        })
        .unwrap();

    // Run the process multiple times, ensure it's not moved in the
    // process.
    let mut process = scheduler.remove().unwrap();
    assert_eq!(process.as_mut().run(&mut ctx), Poll::Pending);
    scheduler.add_back_process(process);

    scheduler.mark_ready(pid);
    let mut process = scheduler.remove().unwrap();
    assert_eq!(process.as_mut().run(&mut ctx), Poll::Pending);
    scheduler.add_back_process(process);

    scheduler.mark_ready(pid);
    let mut process = scheduler.remove().unwrap();
    assert_eq!(process.as_mut().run(&mut ctx), Poll::Pending);
}

#[test]
fn assert_future_process_unmoved() {
    let scheduler = Scheduler::new();
    let waker = nop_task_waker();
    let mut ctx = task::Context::from_waker(&waker);

    let _ = scheduler.add_new_process(Priority::NORMAL, |_| {
        Ok::<_, !>((FutureProcess(AssertUnmoved::new(pending())), ()))
    });

    // Run the process multiple times, ensure it's not moved in the
    // process.
    let mut process = scheduler.remove().unwrap();
    let pid = process.as_ref().id();
    assert_eq!(process.as_mut().run(&mut ctx), Poll::Pending);
    scheduler.add_back_process(process);

    scheduler.mark_ready(pid);
    let mut process = scheduler.remove().unwrap();
    assert_eq!(process.as_mut().run(&mut ctx), Poll::Pending);
    scheduler.add_back_process(process);

    scheduler.mark_ready(pid);
    let mut process = scheduler.remove().unwrap();
    assert_eq!(process.as_mut().run(&mut ctx), Poll::Pending);
}

fn add_test_actor(scheduler: &Scheduler, priority: Priority) -> ProcessId {
    scheduler
        .add_new_process(priority, |pid| {
            let new_actor = simple_actor as fn(_) -> _;
            let rt = ThreadSafe::new(test::shared_internals());
            ActorFuture::new(NoSupervisor, new_actor, (), rt).map(|(future, _)| (future, pid))
        })
        .unwrap()
}
