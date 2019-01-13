//! Tests for the scheduler.

// TODO: test not moving Actor inside ActorProcess, depends on
// https://github.com/rust-lang-nursery/futures-rs/issues/1385.

use std::{io, mem};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{self, AtomicUsize};
use std::time::Duration;

use crossbeam_channel as channel;
use futures_test::future::{AssertUnmoved, FutureTestExt};
use futures_util::future::{empty, Empty};

use crate::actor::{ActorContext, NewActor};
use crate::initiator::Initiator;
use crate::scheduler::process::{Process, ProcessId, ProcessResult};
use crate::scheduler::{Priority, ProcessState, Scheduler};
use crate::supervisor::NoSupervisor;
use crate::system::ActorSystemRef;
use crate::test::{init_actor, system_ref};
use crate::util::Shared;
use crate::waker::new_waker;

fn assert_size<T>(expected: usize) {
    assert_eq!(mem::size_of::<T>(), expected);
}

#[test]
fn size_assertions() {
    assert_size::<ProcessState>(mem::size_of::<Pin<Box<dyn Process>>>());
}

#[derive(Debug)]
struct TestProcess {
    id: ProcessId,
    priority: Priority,
    result: ProcessResult,
}

impl TestProcess {
    fn new(id: ProcessId, priority: Priority, result: ProcessResult) -> Pin<Box<dyn Process>> {
        Box::pin(TestProcess { id, priority, result })
    }
}

impl Process for TestProcess {
    fn id(&self) -> ProcessId {
        self.id
    }

    fn priority(&self) -> Priority {
        self.priority
    }

    fn runtime(&self) -> Duration {
        Duration::from_millis(0)
    }

    fn run(self: Pin<&mut Self>, _system_ref: &mut ActorSystemRef) -> ProcessResult {
        self.result
    }
}

#[test]
fn process_state() {
    let mut process_state = ProcessState::Active;
    assert!(process_state.is_active());

    let process = TestProcess::new(ProcessId(0), Priority::NORMAL, ProcessResult::Complete);
    process_state.mark_inactive(process);
    assert!(!process_state.is_active());

    let process = process_state.mark_active();
    assert!(process_state.is_active());
    assert_eq!(process.id(), ProcessId(0));
    assert_eq!(process.priority(), Priority::NORMAL);
}

#[test]
fn scheduler() {
    let (mut scheduler, mut scheduler_ref) = Scheduler::new();
    let mut system_ref = system_ref();

    // Shouldn't run any process yet, since none are added.
    assert!(!scheduler.process_ready());
    assert!(!scheduler.run_process(&mut system_ref));

    // Scheduling an unknown process should do nothing.
    scheduler.schedule(ProcessId(0));
    assert!(!scheduler.process_ready());
    assert!(!scheduler.run_process(&mut system_ref));

    // Add a process to the scheduler.
    let pid = ProcessId(0);
    let process = TestProcess::new(pid, Priority::NORMAL, ProcessResult::Complete);
    let process_entry = scheduler_ref.add_process();
    assert_eq!(process_entry.pid(), pid);
    process_entry.add_process(pid, process);

    // Newly added processes aren't ready by default.
    assert!(!scheduler.process_ready());
    assert!(!scheduler.run_process(&mut system_ref));

    // After scheduling the process should be ready to run.
    scheduler.schedule(pid);
    assert!(scheduler.process_ready());
    assert!(scheduler.run_process(&mut system_ref));
    // After the process is run, and returned `ProcessResult::Complete`, it
    // should be removed.
    assert!(!scheduler.process_ready());
    assert!(!scheduler.run_process(&mut system_ref));

    // Since the previous process was completed it should be removed, which
    // means the pid will be reused.
    let process = TestProcess::new(pid, Priority::NORMAL, ProcessResult::Pending);
    let process_entry = scheduler_ref.add_process();
    assert_eq!(process_entry.pid(), pid);
    process_entry.add_process(pid, process);

    // Again newly added processes aren't ready by default.
    assert!(!scheduler.process_ready());
    assert!(!scheduler.run_process(&mut system_ref));

    // After scheduling the process should be ready to run.
    scheduler.schedule(pid);
    assert!(scheduler.process_ready());
    assert!(scheduler.run_process(&mut system_ref));
    // Even though the process was not completed it is no longer ready to run.
    assert!(!scheduler.process_ready());
    assert!(!scheduler.run_process(&mut system_ref));
}

#[derive(Debug)]
struct OrderTestProcess {
    id: ProcessId,
    priority: Priority,
    runtime: Duration,
    order: Shared<Vec<usize>>,
}

impl OrderTestProcess {
    fn new(id: ProcessId, priority: Priority, order: Shared<Vec<usize>>) -> Pin<Box<dyn Process>> {
        Box::pin(OrderTestProcess { id, priority, runtime: Duration::from_millis(0), order })
    }
}

impl Process for OrderTestProcess {
    fn id(&self) -> ProcessId {
        self.id
    }

    fn priority(&self) -> Priority {
        self.priority
    }

    fn runtime(&self) -> Duration {
        self.runtime
    }

    fn run(mut self: Pin<&mut Self>, _system_ref: &mut ActorSystemRef) -> ProcessResult {
        let pid = self.id;
        self.order.borrow_mut().push(pid.0);
        self.runtime += Duration::from_millis(10);
        ProcessResult::Pending
    }
}

#[test]
fn scheduler_run_order() {
    let (mut scheduler, mut scheduler_ref) = Scheduler::new();
    let mut system_ref = system_ref();

    // The order in which the processes have been run.
    let run_order = Shared::new(Vec::new());

    // Add our processes.
    let priorities = [Priority::LOW, Priority::NORMAL, Priority::HIGH];
    for priority in priorities.iter() {
        let process_entry = scheduler_ref.add_process();
        let pid = process_entry.pid();
        let process = OrderTestProcess::new(pid, *priority, run_order.clone());
        process_entry.add_process(pid, process);
    }

    // Schedule all processes.
    for pid in 0..3 {
        scheduler.schedule(ProcessId(pid));
    }
    assert!(scheduler.process_ready());

    // Run all processes, should be in order of priority (since there runtimes
    // are equal).
    for _ in 0..3 {
        assert!(scheduler.run_process(&mut system_ref));
    }
    assert!(!scheduler.process_ready());
    assert_eq!(*run_order.borrow(), vec![2, 1, 0]);
}

async fn actor(mut ctx: ActorContext<()>) -> Result<(), !> {
    let _msg = await!(ctx.receive());
    Ok(())
}

#[test]
fn actor_process() {
    let (mut scheduler, mut scheduler_ref) = Scheduler::new();
    let mut system_ref = system_ref();

    // Create our actor.
    #[allow(trivial_casts)]
    let new_actor = actor as fn(_) -> _;
    let (actor, mut actor_ref) = init_actor(new_actor, ()).unwrap();

    // Create the waker.
    let pid = ProcessId(0);
    let (sender, _) = channel::unbounded();
    let waker = new_waker(pid, sender);

    // Add the actor to the scheduler.
    let process_entry = scheduler_ref.add_process();
    let inbox = actor_ref.get_inbox().unwrap();
    process_entry.add_actor(Priority::NORMAL, NoSupervisor, new_actor, actor,
        inbox, waker);

    // Schedule and run, should return Pending and become inactive.
    scheduler.schedule(ProcessId(0));
    assert!(scheduler.run_process(&mut system_ref));
    assert!(!scheduler.process_ready());

    // Send a message to the actor, schedule and run again. This time it should
    // complete.
    actor_ref.send(()).unwrap();
    scheduler.schedule(ProcessId(0));
    assert!(scheduler.run_process(&mut system_ref));

    // Now no processes should be ready.
    scheduler.schedule(ProcessId(0));
    assert!(!scheduler.process_ready());
    assert!(!scheduler.run_process(&mut system_ref));
}

struct TestNewActor;

impl NewActor for TestNewActor {
    type Message = ();
    type Argument = ();
    type Actor = AssertUnmoved<Empty<Result<(), !>>>;
    type Error = !;

    fn new(&mut self, ctx: ActorContext<Self::Message>, _arg: Self::Argument) -> Result<Self::Actor, Self::Error> {
        // In the test we need the access to the inbox, to achieve that we can't
        // drop the context, so we forget about it here leaking the inbox.
        mem::forget(ctx);
        Ok(empty().assert_unmoved())
    }
}

#[test]
fn assert_actor_unmoved() {
    let (mut scheduler, mut scheduler_ref) = Scheduler::new();
    let mut system_ref = system_ref();

    // Create our actor.
    let (actor, mut actor_ref) = init_actor(TestNewActor, ()).unwrap();

    // Create the waker.
    let pid = ProcessId(0);
    let (sender, _) = channel::unbounded();
    let waker = new_waker(pid, sender);

    // Add the actor to the scheduler.
    let process_entry = scheduler_ref.add_process();
    let inbox = actor_ref.get_inbox().unwrap();
    process_entry.add_actor(Priority::NORMAL, NoSupervisor, TestNewActor,
        actor, inbox, waker);

    // Schedule and run the process multiple times, ensure it's not moved in the
    // process.
    scheduler.schedule(ProcessId(0));
    assert!(scheduler.run_process(&mut system_ref));
    scheduler.schedule(ProcessId(0));
    assert!(scheduler.run_process(&mut system_ref));
    scheduler.schedule(ProcessId(0));
    assert!(scheduler.run_process(&mut system_ref));
}

pub struct SimpleInitiator {
    called: Arc<AtomicUsize>,
}

impl Initiator for SimpleInitiator {
    fn poll(&mut self, _: &mut ActorSystemRef) -> io::Result<()> {
        match self.called.fetch_add(1, atomic::Ordering::SeqCst) {
            0 => Ok(()),
            1 => Err(io::ErrorKind::Other.into()),
            _ => unreachable!(),
        }
    }
}

#[test]
fn adding_initiator_process() {
    let (mut scheduler, mut scheduler_ref) = Scheduler::new();
    let mut system_ref = system_ref();

    // Add the initiator to the scheduler.
    let called = Arc::new(AtomicUsize::new(0));
    let initiator = SimpleInitiator { called: Arc::clone(&called) };
    let process_entry = scheduler_ref.add_process();
    process_entry.add_initiator(initiator);

    // Schedule and run, should return Ok and become inactive.
    scheduler.schedule(ProcessId(0));
    assert!(scheduler.run_process(&mut system_ref));
    assert_eq!(called.load(atomic::Ordering::SeqCst), 1);
    assert!(!scheduler.process_ready());

    // Schedule and run again, should return an error this time and be removed.
    scheduler.schedule(ProcessId(0));
    assert!(scheduler.run_process(&mut system_ref));
    assert_eq!(called.load(atomic::Ordering::SeqCst), 2);

    // Now no processes should be ready.
    scheduler.schedule(ProcessId(0));
    assert!(!scheduler.process_ready());
    assert!(!scheduler.run_process(&mut system_ref));
    assert_eq!(called.load(atomic::Ordering::SeqCst), 2);
}
