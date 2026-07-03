//! Tests for the scheduler.

use std::cell::RefCell;
use std::future::pending;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{self, Poll};
use std::thread::sleep;
use std::time::Duration;

use heph::ActorFutureBuilder;
use heph::actor::{self, actor_fn};
use heph::supervisor::NoSupervisor;

use crate::scheduler::{Cfs, LocalScheduler, Process, process};
use crate::setup::scheduler::{FutureTask, Process as _, ProcessId, RunStats, Scheduler, Task};
use crate::spawn::options::Priority;
use crate::test::{self, AssertUnmoved, TestAssertUnmovedNewActor, assert_size};
use crate::{Access, ThreadLocal};

#[test]
fn size_assertions() {
    assert_size::<Priority>(1);
    assert_size::<process::Process<Cfs, Box<dyn Task>>>(40);
    assert_size::<Process<Cfs>>(48);
}

#[derive(Debug)]
struct NopTestTask;

impl Task for NopTestTask {
    fn name(&self) -> &'static str {
        "NopTestTask"
    }
}

impl Future for NopTestTask {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<()> {
        unimplemented!();
    }
}

#[test]
#[allow(clippy::eq_op)] // Need to compare `Process` to itself.
fn process_data_equality() {
    let process1 = Process::<Cfs>::new(ProcessId::new(0), Priority::LOW, Box::pin(NopTestTask));
    let process2 = Process::<Cfs>::new(ProcessId::new(1), Priority::NORMAL, Box::pin(NopTestTask));
    let process3 = Process::<Cfs>::new(ProcessId::new(2), Priority::HIGH, Box::pin(NopTestTask));
    let process4 = Process::<Cfs>::new(ProcessId::new(3), Priority::LOW, Box::pin(NopTestTask));

    // Equality is only based on id alone.
    assert_eq!(process1, process1);
    assert_ne!(process1, process2);
    assert_ne!(process1, process3);
    assert_ne!(process1, process4);

    assert_ne!(process2, process1);
    assert_eq!(process2, process2);
    assert_ne!(process2, process3);
    assert_ne!(process2, process4);

    assert_ne!(process3, process1);
    assert_ne!(process3, process2);
    assert_eq!(process3, process3);
    assert_ne!(process3, process4);

    assert_ne!(process4, process1);
    assert_ne!(process4, process2);
    assert_ne!(process4, process3);
    assert_eq!(process4, process4);
}

#[derive(Debug)]
struct SleepyTask(Duration);

impl Task for SleepyTask {
    fn name(&self) -> &'static str {
        "SleepyTask"
    }
}

impl Future for SleepyTask {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<()> {
        sleep(self.0);
        Poll::Pending
    }
}

#[test]
fn process_data_runtime_increase() {
    const SLEEP_TIME: Duration = Duration::from_millis(10);

    let mut process = Box::pin(Process::<Cfs>::new(
        ProcessId::new(0),
        Priority::HIGH,
        Box::pin(SleepyTask(SLEEP_TIME)),
    ));
    process
        .scheduler_data()
        .set_fair_runtime(Duration::from_millis(10));

    // Runtime must increase after running.
    let waker = task::Waker::noop();
    let mut ctx = task::Context::from_waker(&waker);
    let stats = Pin::new(&mut process).run(&mut ctx);
    assert_eq!(stats.result(), Poll::Pending);
    assert!(process.scheduler_data().fair_runtime() >= SLEEP_TIME);
}

#[test]
fn future_task_assert_future_unmoved() {
    let task = FutureTask(AssertUnmoved::new(pending()));
    let mut task: Pin<Box<dyn Task>> = Box::pin(task);

    // All we do is run it a couple of times, it should panic if the actor is
    // moved.
    let waker = task::Waker::noop();
    let mut ctx = task::Context::from_waker(&waker);
    let res = task.as_mut().poll(&mut ctx);
    assert_eq!(res, Poll::Pending);
    let res = task.as_mut().poll(&mut ctx);
    assert_eq!(res, Poll::Pending);
    let res = task.as_mut().poll(&mut ctx);
    assert_eq!(res, Poll::Pending);
}

#[test]
fn has_process() {
    let mut scheduler = test_scheduler();
    assert!(!scheduler.has_process());
    assert!(!scheduler.has_ready_process());

    let _ = scheduler.add_task(Priority::NORMAL, NopTestTask);
    assert!(scheduler.has_process());
    assert!(scheduler.has_ready_process());
}

async fn simple_actor(_: actor::Context<!, ThreadLocal>) {}

#[test]
fn add_actor() {
    let mut scheduler = test_scheduler();
    let new_actor = actor_fn(simple_actor);
    let rt = test::runtime();
    let (task, _) = ActorFutureBuilder::new()
        .with_rt(rt)
        .build(NoSupervisor, new_actor, ());
    let _ = scheduler.add_task(Priority::NORMAL, task);
    assert!(scheduler.has_process());
    assert!(scheduler.has_ready_process());
}

#[test]
fn wake_process() {
    let mut scheduler = test_scheduler();

    let new_actor = actor_fn(simple_actor);
    let rt = test::runtime();
    let (task, _) = ActorFutureBuilder::new()
        .with_rt(rt)
        .build(NoSupervisor, new_actor, ());
    let _ = scheduler.add_task(Priority::NORMAL, task);

    assert!(scheduler.has_process());
    assert!(scheduler.has_ready_process());

    let (process, waker) = scheduler.next_process().unwrap();
    scheduler.add_back_process(process, RunStats::empty());

    assert!(scheduler.next_process().is_none());
    assert!(scheduler.has_process());
    assert!(!scheduler.has_ready_process());

    waker.wake();
    let _ = scheduler.process_wakeups();

    assert!(scheduler.has_process());
    assert!(scheduler.has_ready_process());
    assert!(scheduler.next_process().is_some());
}

#[test]
fn wake_processafter_completion() {
    let mut scheduler = test_scheduler();

    let _ = add_test_actor(&mut scheduler, Priority::NORMAL);

    assert!(scheduler.has_process());
    assert!(scheduler.has_ready_process());

    let (process, waker) = scheduler.next_process().unwrap();
    scheduler.complete_process(process).unwrap();

    waker.wake(); // This should be fine.
}

#[test]
fn next_process() {
    let mut scheduler = test_scheduler();

    let pid = add_test_actor(&mut scheduler, Priority::NORMAL);

    if let Some((process, _)) = scheduler.next_process() {
        assert_eq!(process.id(), pid);
        assert!(scheduler.has_process());
        assert!(!scheduler.has_ready_process());
    } else {
        panic!("expected a process");
    }
}

#[test]
fn next_process_order() {
    let mut scheduler = test_scheduler();

    let pid1 = add_test_actor(&mut scheduler, Priority::LOW);
    let pid2 = add_test_actor(&mut scheduler, Priority::HIGH);
    let pid3 = add_test_actor(&mut scheduler, Priority::NORMAL);

    assert!(scheduler.has_process());
    assert!(scheduler.has_ready_process());

    // Process 2 has a higher priority, should be scheduled first.
    let (process2, _) = scheduler.next_process().unwrap();
    assert_eq!(process2.id(), pid2);
    let (process3, _) = scheduler.next_process().unwrap();
    assert_eq!(process3.id(), pid3);
    let (process1, _) = scheduler.next_process().unwrap();
    assert_eq!(process1.id(), pid1);

    assert!(process1 < process2);
    assert!(process1 < process3);
    assert!(process2 > process1);
    assert!(process2 > process3);
    assert!(process3 > process1);
    assert!(process3 < process2);

    assert!(scheduler.next_process().is_none());
}

#[test]
fn add_back_process() {
    let mut scheduler = test_scheduler();

    let pid = add_test_actor(&mut scheduler, Priority::NORMAL);

    assert!(scheduler.has_process());
    assert!(scheduler.has_ready_process());

    let (process, waker) = scheduler.next_process().unwrap();

    // After adding the process back it's not ready.
    scheduler.add_back_process(process, RunStats::empty());
    assert!(scheduler.has_process());
    assert!(!scheduler.has_ready_process());

    // After waking it's still not ready.
    waker.wake();
    assert!(scheduler.has_process());
    assert!(!scheduler.has_ready_process());

    // Only when we mark all processes as ready is it ready.
    let _ = scheduler.process_wakeups();
    assert!(scheduler.has_process());
    assert!(scheduler.has_ready_process());

    let (process, _) = scheduler.next_process().unwrap();
    assert_eq!(process.id(), pid);
}

#[test]
fn add_back_process_awoken() {
    let mut scheduler = test_scheduler();

    let _ = add_test_actor(&mut scheduler, Priority::NORMAL);

    let (process, waker) = scheduler.next_process().unwrap();

    // Waking it before it's ready
    waker.wake();

    // After adding the process back it's not ready, even though it got awoken
    // above.
    scheduler.add_back_process(process, RunStats::empty());
    assert!(scheduler.has_process());
    assert!(!scheduler.has_ready_process());
}

// NOTE: This is here because we don't really care about the elapsed duration in
// these tests, so this makes them easier to write.
impl PartialEq<Poll<()>> for RunStats {
    fn eq(&self, other: &Poll<()>) -> bool {
        self.result().eq(other)
    }
}

#[test]
fn scheduler_run_order() {
    async fn order_actor(
        _: actor::Context<!, ThreadLocal>,
        id: usize,
        order: Rc<RefCell<Vec<usize>>>,
    ) {
        order.borrow_mut().push(id);
    }

    let mut scheduler = test_scheduler();
    let waker = task::Waker::noop();
    let mut ctx = task::Context::from_waker(&waker);

    // The order in which the processes have been run.
    let run_order = Rc::new(RefCell::new(Vec::new()));

    // Add our processes.
    let new_actor = actor_fn(order_actor);
    let priorities = [Priority::LOW, Priority::NORMAL, Priority::HIGH];
    let mut pids = vec![];
    for (id, priority) in priorities.iter().enumerate() {
        let rt = test::runtime();
        let (task, _) = ActorFutureBuilder::new().with_rt(rt).build(
            NoSupervisor,
            new_actor,
            (id, run_order.clone()),
        );
        let pid = scheduler.add_task(*priority, task);
        pids.push(pid);
    }

    assert!(scheduler.has_process());
    assert!(scheduler.has_ready_process());

    // Run all processes, should be in order of priority (since there runtimes
    // are equal).
    for _ in 0..3 {
        let (mut process, _) = scheduler.next_process().unwrap();
        let stats = Pin::new(&mut process).run(&mut ctx);
        assert_eq!(stats.result(), Poll::Ready(()));
        scheduler.complete_process(process).unwrap();
    }
    assert!(!scheduler.has_process());
    assert_eq!(*run_order.borrow(), vec![2_usize, 1, 0]);
}

#[test]
fn assert_actor_process_unmoved() {
    let mut scheduler = test_scheduler();

    let rt = test::runtime();
    let (task, _) = ActorFutureBuilder::new().with_rt(rt).build(
        NoSupervisor,
        TestAssertUnmovedNewActor::new(),
        (),
    );
    _ = scheduler.add_task(Priority::NORMAL, task);

    // Run the process multiple times, ensure it's not moved in the process.
    let (mut process, waker) = scheduler.next_process().unwrap();
    let mut ctx = task::Context::from_waker(&waker);
    let stats = Pin::new(&mut process).run(&mut ctx);
    assert_eq!(stats.result(), Poll::Pending);
    scheduler.add_back_process(process, stats);

    waker.wake();
    let _ = scheduler.process_wakeups();
    let (mut process, waker) = scheduler.next_process().unwrap();
    let mut ctx = task::Context::from_waker(&waker);
    let stats = Pin::new(&mut process).run(&mut ctx);
    assert_eq!(stats.result(), Poll::Pending);
    scheduler.add_back_process(process, stats);

    waker.wake();
    let _ = scheduler.process_wakeups();
    let (mut process, waker) = scheduler.next_process().unwrap();
    let mut ctx = task::Context::from_waker(&waker);
    let stats = Pin::new(&mut process).run(&mut ctx);
    assert_eq!(stats.result(), Poll::Pending);
}

#[test]
fn assert_future_process_unmoved() {
    let mut scheduler = test_scheduler();

    let task = FutureTask(AssertUnmoved::new(pending()));
    let _ = scheduler.add_task(Priority::NORMAL, task);

    // Run the process multiple times, ensure it's not moved in the process.
    let (mut process, waker) = scheduler.next_process().unwrap();
    let mut ctx = task::Context::from_waker(&waker);
    let stats = Pin::new(&mut process).run(&mut ctx);
    assert_eq!(stats.result(), Poll::Pending);
    scheduler.add_back_process(process, stats);

    waker.wake();
    let _ = scheduler.process_wakeups();
    let (mut process, waker) = scheduler.next_process().unwrap();
    let mut ctx = task::Context::from_waker(&waker);
    let stats = Pin::new(&mut process).run(&mut ctx);
    assert_eq!(stats.result(), Poll::Pending);
    scheduler.add_back_process(process, stats);

    drop(ctx);
    waker.wake();
    let _ = scheduler.process_wakeups();
    let (mut process, waker) = scheduler.next_process().unwrap();
    let mut ctx = task::Context::from_waker(&waker);
    let stats = Pin::new(&mut process).run(&mut ctx);
    assert_eq!(stats.result(), Poll::Pending);
}

fn add_test_actor(scheduler: &mut LocalScheduler<Cfs>, priority: Priority) -> ProcessId {
    let new_actor = actor_fn(simple_actor);
    let rt = test::runtime();
    let (task, _) = ActorFutureBuilder::new()
        .with_rt(rt)
        .build(NoSupervisor, new_actor, ());
    scheduler.add_task(priority, task)
}

/// Creates a `LocalScheduler` connected to the test runtime.
fn test_scheduler() -> LocalScheduler<Cfs> {
    let rt = test::runtime();
    LocalScheduler::new(rt.sq())
}
