//! Tests for the scheduler.

use std::cmp::Ordering;
use std::mem::{self, forget};
use std::pin::Pin;
use std::thread::sleep;
use std::time::Duration;

use futures_test::future::{AssertUnmoved, FutureTestExt};
use futures_util::future::{empty, Empty};
use futures_util::pending;
use gaea::event::{self, Sink};
use gaea::{Event, Ready};

use crate::supervisor::NoSupervisor;
use crate::system::process::{Process, ProcessId, ProcessResult};
use crate::system::scheduler::{Priority, ProcessData, ProcessState, Scheduler};
use crate::system::ActorSystemRef;
use crate::test::{init_actor, system_ref};
use crate::util::Shared;
use crate::{actor, NewActor};

fn assert_size<T>(expected: usize) {
    assert_eq!(mem::size_of::<T>(), expected);
}

#[test]
fn size_assertions() {
    assert_size::<ProcessId>(4);
    assert_size::<Priority>(1);
    assert_size::<Duration>(16);
    assert_size::<Box<dyn Process>>(16);
    assert_size::<ProcessData>(40);

    assert_size::<ProcessState>(mem::size_of::<ProcessData>());
}

#[test]
fn priority() {
    assert!(Priority::HIGH > Priority::NORMAL);
    assert!(Priority::NORMAL > Priority::LOW);
    assert!(Priority::HIGH > Priority::LOW);

    assert_eq!(Priority::HIGH, Priority::HIGH);
    assert_ne!(Priority::HIGH, Priority::NORMAL);

    assert_eq!(Priority::default(), Priority::NORMAL);
}

#[test]
fn priority_duration_multiplication() {
    let duration = Duration::from_millis(1);
    let high = duration * Priority::HIGH;
    let normal = duration * Priority::NORMAL;
    let low = duration * Priority::LOW;

    assert!(high < normal);
    assert!(normal < low);
    assert!(high < low);
}

#[derive(Debug)]
struct NopTestProcess;

impl Process for NopTestProcess {
    fn run(
        self: Pin<&mut Self>,
        _system_ref: &mut ActorSystemRef,
        _pid: ProcessId,
    ) -> ProcessResult {
        unimplemented!();
    }
}

#[test]
fn process_data_equality() {
    let process1 = ProcessData {
        id: ProcessId(0),
        priority: Priority::LOW,
        runtime: Duration::from_millis(0),
        process: Box::pin(NopTestProcess),
    };
    let process2 = ProcessData {
        id: ProcessId(0),
        priority: Priority::NORMAL,
        runtime: Duration::from_millis(0),
        process: Box::pin(NopTestProcess),
    };
    let process3 = ProcessData {
        id: ProcessId(1),
        priority: Priority::HIGH,
        runtime: Duration::from_millis(0),
        process: Box::pin(NopTestProcess),
    };

    // Equality is only based on id alone.
    assert_eq!(process1, process1);
    assert_eq!(process2, process2);
    assert_eq!(process3, process3);
    assert_eq!(process1, process2);
    assert_ne!(process1, process3);
    assert_ne!(process2, process3);
    assert_ne!(process2, process3);
}

#[test]
fn process_data_ordering() {
    let mut process1 = ProcessData {
        id: ProcessId(0),
        priority: Priority::HIGH,
        runtime: Duration::from_millis(10),
        process: Box::pin(NopTestProcess),
    };
    let mut process2 = ProcessData {
        id: ProcessId(0),
        priority: Priority::NORMAL,
        runtime: Duration::from_millis(10),
        process: Box::pin(NopTestProcess),
    };
    let mut process3 = ProcessData {
        id: ProcessId(1),
        priority: Priority::LOW,
        runtime: Duration::from_millis(10),
        process: Box::pin(NopTestProcess),
    };

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
    process1.runtime = duration;
    process2.runtime = duration;
    process3.runtime = duration;

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
    fn run(
        self: Pin<&mut Self>,
        _system_ref: &mut ActorSystemRef,
        _pid: ProcessId,
    ) -> ProcessResult {
        sleep(self.0);
        ProcessResult::Pending
    }
}

#[test]
fn process_data_runtime_increase() {
    const SLEEP_TIME: Duration = Duration::from_millis(10);

    let mut process = ProcessData {
        id: ProcessId(0),
        priority: Priority::HIGH,
        runtime: Duration::from_millis(10),
        process: Box::pin(SleepyProcess(SLEEP_TIME)),
    };

    // Runtime must increase after running.
    let mut system_ref = system_ref();
    let res = process.run(&mut system_ref);
    assert_eq!(res, ProcessResult::Pending);
    assert!(process.runtime >= SLEEP_TIME);
}

#[test]
fn process_state() {
    let process = ProcessData {
        id: ProcessId(0),
        priority: Priority::HIGH,
        runtime: Duration::from_millis(10),
        process: Box::pin(NopTestProcess),
    };

    let mut process_state = ProcessState::Inactive(process);
    assert!(!process_state.is_active());

    let process = process_state.mark_active();
    assert!(process_state.is_active());

    process_state.mark_inactive(process);
    assert!(!process_state.is_active());
}

async fn simple_actor(_ctx: actor::Context<!>) -> Result<(), !> {
    Ok(())
}

#[test]
fn adding_process() {
    let (mut scheduler, mut scheduler_ref) = Scheduler::new();
    let mut system_ref = system_ref();

    // Shouldn't run any process yet, since none are added.
    assert!(scheduler.is_empty());
    assert!(!scheduler.has_active_process());
    assert!(!scheduler.run_process(&mut system_ref));

    // Add an actor to the scheduler.
    let process_entry = scheduler_ref.add_process();
    assert_eq!(process_entry.pid(), ProcessId(0));
    #[allow(trivial_casts)]
    let new_actor = simple_actor as fn(_) -> _;
    let (actor, mut actor_ref) = init_actor(new_actor, ()).unwrap();
    let inbox = actor_ref.get_inbox().unwrap();
    process_entry.add_actor(Priority::NORMAL, NoSupervisor, new_actor, actor, inbox);

    // Newly added processes aren't ready by default.
    assert!(!scheduler.is_empty());
    assert!(!scheduler.has_active_process());
    assert!(!scheduler.run_process(&mut system_ref));

    // After scheduling the process should be ready to run.
    scheduler.schedule(ProcessId(0));
    assert!(!scheduler.is_empty());
    assert!(scheduler.has_active_process());
    assert!(scheduler.run_process(&mut system_ref));
    // After the process is run, and returned `ProcessResult::Complete`, it
    // should be removed.
    assert!(scheduler.is_empty());
    assert!(!scheduler.has_active_process());
    assert!(!scheduler.run_process(&mut system_ref));
    assert!(scheduler.is_empty());
    assert!(!scheduler.has_active_process());
}

#[test]
fn adding_process_reusing_pid() {
    let (mut scheduler, mut scheduler_ref) = Scheduler::new();
    let mut system_ref = system_ref();

    // Add an actor to the scheduler.
    let process_entry = scheduler_ref.add_process();
    assert_eq!(process_entry.pid(), ProcessId(0));
    #[allow(trivial_casts)]
    let new_actor = simple_actor as fn(_) -> _;
    let (actor, mut actor_ref) = init_actor(new_actor, ()).unwrap();
    let inbox = actor_ref.get_inbox().unwrap();
    process_entry.add_actor(Priority::NORMAL, NoSupervisor, new_actor, actor, inbox);

    scheduler.schedule(ProcessId(0));
    assert!(scheduler.run_process(&mut system_ref));

    // Since the previous process was completed it should be removed, which
    // means the pid will be available for reuse.
    let process_entry = scheduler_ref.add_process();
    assert_eq!(process_entry.pid(), ProcessId(0));
    let (actor, mut actor_ref) = init_actor(new_actor, ()).unwrap();
    let inbox = actor_ref.get_inbox().unwrap();
    process_entry.add_actor(Priority::NORMAL, NoSupervisor, new_actor, actor, inbox);

    // Again newly added processes aren't ready by default.
    assert!(!scheduler.is_empty());
    assert!(!scheduler.has_active_process());
    assert!(!scheduler.run_process(&mut system_ref));

    // After scheduling the process should be ready to run.
    scheduler.schedule(ProcessId(0));
    assert!(!scheduler.is_empty());
    assert!(scheduler.has_active_process());
    assert!(scheduler.run_process(&mut system_ref));
}

#[test]
fn scheduling_unknown_proccess() {
    let (mut scheduler, _) = Scheduler::new();
    let mut system_ref = system_ref();

    assert!(scheduler.is_empty());
    assert!(!scheduler.has_active_process());
    assert!(!scheduler.run_process(&mut system_ref));

    // Scheduling an unknown process should do nothing.
    scheduler.schedule(ProcessId(0));
    assert!(scheduler.is_empty());
    assert!(!scheduler.has_active_process());
    assert!(!scheduler.run_process(&mut system_ref));
}

async fn pending_actor(_ctx: actor::Context<!>) -> Result<(), !> {
    pending!();
    Ok(())
}

#[test]
fn running_process() {
    let (mut scheduler, mut scheduler_ref) = Scheduler::new();
    let mut system_ref = system_ref();

    // Shouldn't run any process yet, since none are added.
    assert!(scheduler.is_empty());
    assert!(!scheduler.has_active_process());
    assert!(!scheduler.run_process(&mut system_ref));

    // Add an actor to the scheduler.
    let process_entry = scheduler_ref.add_process();
    assert_eq!(process_entry.pid(), ProcessId(0));
    #[allow(trivial_casts)]
    let new_actor = pending_actor as fn(_) -> _;
    let (actor, mut actor_ref) = init_actor(new_actor, ()).unwrap();
    let inbox = actor_ref.get_inbox().unwrap();
    process_entry.add_actor(Priority::NORMAL, NoSupervisor, new_actor, actor, inbox);

    // Newly added processes aren't ready by default.
    assert!(!scheduler.is_empty());
    assert!(!scheduler.has_active_process());
    assert!(!scheduler.run_process(&mut system_ref));

    // After scheduling the process should be ready to run.
    scheduler.schedule(ProcessId(0));
    assert!(!scheduler.is_empty());
    assert!(scheduler.has_active_process());
    assert!(scheduler.run_process(&mut system_ref));

    // Process should return pending, making it inactive.
    assert!(!scheduler.is_empty());
    assert!(!scheduler.has_active_process());
    assert!(!scheduler.run_process(&mut system_ref));

    scheduler.schedule(ProcessId(0));
    assert!(scheduler.run_process(&mut system_ref));
    assert!(scheduler.is_empty());
    assert!(!scheduler.has_active_process());
}

#[test]
fn event_sink_impl() {
    let (mut scheduler, mut scheduler_ref) = Scheduler::new();
    let mut system_ref = system_ref();

    // Add an actor to the scheduler.
    let process_entry = scheduler_ref.add_process();
    assert_eq!(process_entry.pid(), ProcessId(0));
    #[allow(trivial_casts)]
    let new_actor = simple_actor as fn(_) -> _;
    let (actor, mut actor_ref) = init_actor(new_actor, ()).unwrap();
    let inbox = actor_ref.get_inbox().unwrap();
    process_entry.add_actor(Priority::NORMAL, NoSupervisor, new_actor, actor, inbox);

    scheduler.add(Event::new(event::Id(0), Ready::READABLE));
    assert!(scheduler.run_process(&mut system_ref));

    assert_eq!(scheduler.capacity_left(), event::Capacity::Growable);
}

async fn order_actor(
    _ctx: actor::Context<!>,
    id: usize,
    mut order: Shared<Vec<usize>>,
) -> Result<(), !> {
    order.borrow_mut().push(id);
    Ok(())
}

#[test]
fn scheduler_run_order() {
    let (mut scheduler, mut scheduler_ref) = Scheduler::new();
    let mut system_ref = system_ref();

    // The order in which the processes have been run.
    let run_order = Shared::new(Vec::new());

    // Add our processes.
    #[allow(trivial_casts)]
    let new_actor = order_actor as fn(_, _, _) -> _;
    let priorities = [Priority::LOW, Priority::NORMAL, Priority::HIGH];
    for (id, priority) in priorities.iter().enumerate() {
        let process_entry = scheduler_ref.add_process();
        assert_eq!(process_entry.pid(), ProcessId(id as u32));
        let (actor, mut actor_ref) = init_actor(new_actor, (id, run_order.clone())).unwrap();
        let inbox = actor_ref.get_inbox().unwrap();
        process_entry.add_actor(*priority, NoSupervisor, new_actor, actor, inbox);
    }

    // Schedule all processes.
    for pid in 0..3 {
        scheduler.schedule(ProcessId(pid));
    }
    assert!(!scheduler.is_empty());
    assert!(scheduler.has_active_process());

    // Run all processes, should be in order of priority (since there runtimes
    // are equal).
    for _ in 0..3 {
        assert!(scheduler.run_process(&mut system_ref));
    }
    assert!(scheduler.is_empty());
    assert_eq!(*run_order.borrow(), vec![2usize, 1, 0]);
}

#[derive(Copy, Clone)]
struct TestAssertUnmovedNewActor;

impl NewActor for TestAssertUnmovedNewActor {
    type Message = ();
    type Argument = ();
    type Actor = AssertUnmoved<Empty<Result<(), !>>>;
    type Error = !;

    fn new(
        &mut self,
        ctx: actor::Context<Self::Message>,
        _arg: Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        // In the test we need the access to the inbox, to achieve that we can't
        // drop the context, so we forget about it here leaking the inbox.
        forget(ctx);
        Ok(empty().assert_unmoved())
    }
}

#[test]
fn assert_process_unmoved() {
    let (mut scheduler, mut scheduler_ref) = Scheduler::new();
    let mut system_ref = system_ref();

    let new_actor = TestAssertUnmovedNewActor;
    let (actor, mut actor_ref) = init_actor(new_actor, ()).unwrap();

    let process_entry = scheduler_ref.add_process();
    let inbox = actor_ref.get_inbox().unwrap();
    process_entry.add_actor(Priority::NORMAL, NoSupervisor, new_actor, actor, inbox);

    // Schedule and run the process multiple times, ensure it's not moved in the
    // process.
    let pid = ProcessId(0);
    scheduler.schedule(pid);
    assert!(scheduler.run_process(&mut system_ref));
    scheduler.schedule(pid);
    assert!(scheduler.run_process(&mut system_ref));
    scheduler.schedule(pid);
    assert!(scheduler.run_process(&mut system_ref));
}
