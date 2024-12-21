//! Tests for the scheduler.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::future::{pending, Future};
use std::pin::Pin;
use std::rc::Rc;
use std::task::{self, Poll};
use std::thread::sleep;
use std::time::Duration;

use heph::actor::{self, actor_fn};
use heph::supervisor::NoSupervisor;
use heph::ActorFutureBuilder;

use crate::scheduler::process::{self, FutureProcess, Process, RunStats};
use crate::scheduler::{ProcessData, ProcessId, Scheduler};
use crate::spawn::options::Priority;
use crate::test::{self, assert_size, AssertUnmoved, TestAssertUnmovedNewActor};
use crate::worker::SYSTEM_ACTORS;
use crate::ThreadLocal;

#[test]
fn size_assertions() {
    assert_size::<ProcessId>(8);
    assert_size::<Priority>(1);
    assert_size::<process::ProcessData<Box<dyn Process>>>(32);
    assert_size::<ProcessData>(40);
}

#[derive(Debug)]
struct NopTestProcess;

impl Future for NopTestProcess {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<()> {
        unimplemented!();
    }
}

impl Process for NopTestProcess {
    fn name(&self) -> &'static str {
        "NopTestProcess"
    }
}

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

impl Future for SleepyProcess {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<()> {
        sleep(self.0);
        Poll::Pending
    }
}

impl Process for SleepyProcess {
    fn name(&self) -> &'static str {
        "SleepyProcess"
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
    let waker = task::Waker::noop();
    let mut ctx = task::Context::from_waker(&waker);
    let res = process.as_mut().run(&mut ctx);
    assert_eq!(res, Poll::Pending);
    assert!(process.fair_runtime >= SLEEP_TIME);
}

#[test]
fn future_process_assert_future_unmoved() {
    let process = FutureProcess(AssertUnmoved::new(pending()));
    let mut process: Pin<Box<dyn Process>> = Box::pin(process);

    // All we do is run it a couple of times, it should panic if the actor is
    // moved.
    let waker = task::Waker::noop();
    let mut ctx = task::Context::from_waker(&waker);
    let res = process.as_mut().poll(&mut ctx);
    assert_eq!(res, Poll::Pending);
    let res = process.as_mut().poll(&mut ctx);
    assert_eq!(res, Poll::Pending);
    let res = process.as_mut().poll(&mut ctx);
    assert_eq!(res, Poll::Pending);
}

#[test]
fn has_user_process() {
    let mut scheduler = test_scheduler();
    assert!(!scheduler.has_user_process());
    assert!(!scheduler.has_ready_process());

    let _ = scheduler.add_new_process(Priority::NORMAL, FutureProcess(NopTestProcess));
    assert!(scheduler.has_user_process());
    assert!(scheduler.has_ready_process());
}

async fn simple_actor(_: actor::Context<!, ThreadLocal>) {}

#[test]
fn add_actor() {
    let mut scheduler = test_scheduler();
    let new_actor = actor_fn(simple_actor);
    let rt = ThreadLocal::new(test::runtime());
    let (process, _) = ActorFutureBuilder::new()
        .with_rt(rt)
        .build(NoSupervisor, new_actor, ())
        .unwrap();
    let _ = scheduler.add_new_process(Priority::NORMAL, process);
    assert!(scheduler.has_user_process());
    assert!(scheduler.has_ready_process());
}

#[test]
fn mark_ready() {
    let mut scheduler = test_scheduler();

    // Incorrect (outdated) pid should be ok.
    scheduler.mark_ready(ProcessId(100));

    let new_actor = actor_fn(simple_actor);
    let rt = ThreadLocal::new(test::runtime());
    let (process, _) = ActorFutureBuilder::new()
        .with_rt(rt)
        .build(NoSupervisor, new_actor, ())
        .unwrap();
    let pid = scheduler.add_new_process(Priority::NORMAL, process);

    assert!(scheduler.has_user_process());
    assert!(scheduler.has_ready_process());

    let process = scheduler.next_process().unwrap();
    scheduler.add_back_process(process);
    scheduler.mark_ready(pid);
}

#[test]
fn mark_ready_before_run() {
    let mut scheduler = test_scheduler();

    // Incorrect (outdated) pid should be ok.
    scheduler.mark_ready(ProcessId(100));

    let pid = add_test_actor(&mut scheduler, Priority::NORMAL);

    assert!(scheduler.has_user_process());
    assert!(scheduler.has_ready_process());

    let process = scheduler.next_process().unwrap();
    scheduler.mark_ready(pid);
    scheduler.add_back_process(process);
}

#[test]
fn next_process() {
    let mut scheduler = test_scheduler();

    let pid = add_test_actor(&mut scheduler, Priority::NORMAL);

    if let Some(process) = scheduler.next_process() {
        assert_eq!(process.as_ref().id(), pid);
        assert!(!scheduler.has_user_process());
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

    assert!(scheduler.has_user_process());
    assert!(scheduler.has_ready_process());

    // Process 2 has a higher priority, should be scheduled first.
    let process2 = scheduler.next_process().unwrap();
    assert_eq!(process2.as_ref().id(), pid2);
    let process3 = scheduler.next_process().unwrap();
    assert_eq!(process3.as_ref().id(), pid3);
    let process1 = scheduler.next_process().unwrap();
    assert_eq!(process1.as_ref().id(), pid1);

    assert!(process1 < process2);
    assert!(process1 < process3);
    assert!(process2 > process1);
    assert!(process2 > process3);
    assert!(process3 > process1);
    assert!(process3 < process2);

    assert_eq!(scheduler.next_process(), None);
}

#[test]
fn add_process() {
    let mut scheduler = test_scheduler();

    let pid = add_test_actor(&mut scheduler, Priority::NORMAL);

    assert!(scheduler.has_user_process());
    assert!(scheduler.has_ready_process());

    scheduler.mark_ready(pid);
    assert!(scheduler.has_user_process());
    assert!(scheduler.has_ready_process());
    let process = scheduler.next_process().unwrap();
    assert_eq!(process.as_ref().id(), pid);
}

#[test]
fn add_process_marked_ready() {
    let mut scheduler = test_scheduler();

    let pid = add_test_actor(&mut scheduler, Priority::NORMAL);

    let process = scheduler.next_process().unwrap();
    scheduler.add_back_process(process);
    assert!(scheduler.has_user_process());
    assert!(!scheduler.has_ready_process());

    scheduler.mark_ready(pid);
    assert!(scheduler.has_user_process());
    assert!(scheduler.has_ready_process());
    let process = scheduler.next_process().unwrap();
    assert_eq!(process.as_ref().id(), pid);
}

// NOTE: This is here because we don't really care about the elapsed duration in
// these tests, so this makes them easier to write.
impl PartialEq<Poll<()>> for RunStats {
    fn eq(&self, other: &Poll<()>) -> bool {
        self.result.eq(other)
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
        let rt = ThreadLocal::new(test::runtime());
        let (process, _) = ActorFutureBuilder::new()
            .with_rt(rt)
            .build(NoSupervisor, new_actor, (id, run_order.clone()))
            .unwrap();
        let pid = scheduler.add_new_process(*priority, process);
        pids.push(pid);
    }

    assert!(scheduler.has_user_process());
    assert!(scheduler.has_ready_process());

    // Run all processes, should be in order of priority (since there runtimes
    // are equal).
    for _ in 0..3 {
        let mut process = scheduler.next_process().unwrap();
        assert_eq!(process.as_mut().run(&mut ctx), Poll::Ready(()));
    }
    assert!(!scheduler.has_user_process());
    assert_eq!(*run_order.borrow(), vec![2_usize, 1, 0]);
}

#[test]
fn assert_actor_process_unmoved() {
    let mut scheduler = test_scheduler();
    let waker = task::Waker::noop();
    let mut ctx = task::Context::from_waker(&waker);

    let rt = ThreadLocal::new(test::runtime());
    let (process, _) = ActorFutureBuilder::new()
        .with_rt(rt)
        .build(NoSupervisor, TestAssertUnmovedNewActor::new(), ())
        .unwrap();
    let pid = scheduler.add_new_process(Priority::NORMAL, process);

    // Run the process multiple times, ensure it's not moved in the process.
    let mut process = scheduler.next_process().unwrap();
    assert_eq!(process.as_mut().run(&mut ctx), Poll::Pending);
    scheduler.add_back_process(process);

    scheduler.mark_ready(pid);
    let mut process = scheduler.next_process().unwrap();
    assert_eq!(process.as_mut().run(&mut ctx), Poll::Pending);
    scheduler.add_back_process(process);

    scheduler.mark_ready(pid);
    let mut process = scheduler.next_process().unwrap();
    assert_eq!(process.as_mut().run(&mut ctx), Poll::Pending);
}

#[test]
fn assert_future_process_unmoved() {
    let mut scheduler = test_scheduler();
    let waker = task::Waker::noop();
    let mut ctx = task::Context::from_waker(&waker);

    let process = FutureProcess(AssertUnmoved::new(pending()));
    let _ = scheduler.add_new_process(Priority::NORMAL, process);

    // Run the process multiple times, ensure it's not moved in the process.
    let mut process = scheduler.next_process().unwrap();
    let pid = process.as_ref().id();
    assert_eq!(process.as_mut().run(&mut ctx), Poll::Pending);
    scheduler.add_back_process(process);

    scheduler.mark_ready(pid);
    let mut process = scheduler.next_process().unwrap();
    assert_eq!(process.as_mut().run(&mut ctx), Poll::Pending);
    scheduler.add_back_process(process);

    scheduler.mark_ready(pid);
    let mut process = scheduler.next_process().unwrap();
    assert_eq!(process.as_mut().run(&mut ctx), Poll::Pending);
}

fn add_test_actor(scheduler: &mut Scheduler, priority: Priority) -> ProcessId {
    let new_actor = actor_fn(simple_actor);
    let rt = ThreadLocal::new(test::runtime());
    let (process, _) = ActorFutureBuilder::new()
        .with_rt(rt)
        .build(NoSupervisor, new_actor, ())
        .unwrap();
    scheduler.add_new_process(priority, process)
}

/// Creates a `Scheduler` with `SYSTEM_ACTORS` number of fake system actors.
fn test_scheduler() -> Scheduler {
    async fn fake_system_actor(_: actor::Context<!, ThreadLocal>) {
        pending().await
    }

    let mut scheduler = Scheduler::new();
    let new_actor = actor_fn(fake_system_actor);
    let rt = ThreadLocal::new(test::runtime());
    for _ in 0..SYSTEM_ACTORS {
        let (process, _) = ActorFutureBuilder::new()
            .with_rt(rt.clone())
            .build(NoSupervisor, new_actor, ())
            .unwrap();
        let process = Box::pin(ProcessData::new(Priority::SYSTEM, Box::pin(process)));
        scheduler.inactive.add(process);
    }
    scheduler
}
