//! Tests for the scheduler.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::mem::{self, forget};
use std::pin::Pin;
use std::rc::Rc;
use std::thread::sleep;
use std::time::Duration;

use futures_test::future::{AssertUnmoved, FutureTestExt};
use futures_util::future::{pending, Pending};

use crate::rt::process::{Process, ProcessId, ProcessResult};
use crate::rt::scheduler::{Priority, ProcessData, Scheduler};
use crate::rt::RuntimeRef;
use crate::supervisor::NoSupervisor;
use crate::test::{self, init_actor_inbox};
use crate::{actor, NewActor};

fn assert_size<T>(expected: usize) {
    assert_eq!(mem::size_of::<T>(), expected);
}

#[test]
fn size_assertions() {
    assert_size::<ProcessId>(8);
    assert_size::<Priority>(1);
    assert_size::<ProcessData>(40);
}

#[derive(Debug)]
struct NopTestProcess;

impl Process for NopTestProcess {
    fn run(self: Pin<&mut Self>, _: &mut RuntimeRef, _: ProcessId) -> ProcessResult {
        unimplemented!();
    }
}

#[test]
fn process_data_equality() {
    let process1 = ProcessData {
        priority: Priority::LOW,
        fair_runtime: Duration::from_millis(0),
        process: Box::pin(NopTestProcess),
    };
    let process2 = ProcessData {
        priority: Priority::NORMAL,
        fair_runtime: Duration::from_millis(0),
        process: Box::pin(NopTestProcess),
    };
    let process3 = ProcessData {
        priority: Priority::HIGH,
        fair_runtime: Duration::from_millis(0),
        process: Box::pin(NopTestProcess),
    };

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
    let mut process1 = ProcessData {
        priority: Priority::HIGH,
        fair_runtime: Duration::from_millis(10),
        process: Box::pin(NopTestProcess),
    };
    let mut process2 = ProcessData {
        priority: Priority::NORMAL,
        fair_runtime: Duration::from_millis(10),
        process: Box::pin(NopTestProcess),
    };
    let mut process3 = ProcessData {
        priority: Priority::LOW,
        fair_runtime: Duration::from_millis(10),
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
    fn run(self: Pin<&mut Self>, _runtime_ref: &mut RuntimeRef, _pid: ProcessId) -> ProcessResult {
        sleep(self.0);
        ProcessResult::Pending
    }
}

#[test]
fn process_data_runtime_increase() {
    const SLEEP_TIME: Duration = Duration::from_millis(10);

    let mut process = Box::pin(ProcessData {
        priority: Priority::HIGH,
        fair_runtime: Duration::from_millis(10),
        process: Box::pin(SleepyProcess(SLEEP_TIME)),
    });

    // Runtime must increase after running.
    let mut runtime_ref = test::runtime();
    let res = process.as_mut().run(&mut runtime_ref);
    assert_eq!(res, ProcessResult::Pending);
    assert!(process.fair_runtime >= SLEEP_TIME);
}

async fn simple_actor(_ctx: actor::LocalContext<!>) -> Result<(), !> {
    Ok(())
}

#[test]
fn adding_actor() {
    let (mut scheduler, _) = Scheduler::new();

    // Shouldn't run any process yet, since none are added.
    assert!(!scheduler.has_process());
    assert!(!scheduler.has_ready_process());
    assert_eq!(scheduler.next_process(), None);

    // Add an actor to the scheduler.
    let actor_entry = scheduler.add_actor();
    let pid = actor_entry.pid();
    #[allow(trivial_casts)]
    let new_actor = simple_actor as fn(_) -> _;
    let (actor, inbox, inbox_ref) = init_actor_inbox(new_actor, ()).unwrap();
    actor_entry.add(
        Priority::NORMAL,
        NoSupervisor,
        new_actor,
        actor,
        inbox,
        inbox_ref,
    );

    // Newly added processes aren't ready by default.
    assert!(scheduler.has_process());
    assert!(!scheduler.has_ready_process());
    assert_eq!(scheduler.next_process(), None);

    // After scheduling the process should be ready to run.
    scheduler.mark_ready(pid);
    assert!(scheduler.has_process());
    assert!(scheduler.has_ready_process());
    let process = scheduler.next_process().unwrap();
    assert_eq!(process.as_ref().id(), pid);

    // After the process is run, and returned `ProcessResult::Complete`, it
    // should be removed.
    assert!(!scheduler.has_process());
    assert!(!scheduler.has_ready_process());
    assert_eq!(scheduler.next_process(), None);
    assert!(!scheduler.has_process());
    assert!(!scheduler.has_ready_process());

    // Adding the process back means its not ready.
    scheduler.add_process(process);
    assert!(scheduler.has_process());
    assert!(!scheduler.has_ready_process());
    assert_eq!(scheduler.next_process(), None);

    // Marking the same process as ready again.
    scheduler.mark_ready(pid);
    assert!(scheduler.has_process());
    assert!(scheduler.has_ready_process());
    let process = scheduler.next_process().unwrap();
    assert_eq!(process.as_ref().id(), pid);
}

#[test]
fn marking_unknown_pid_as_ready() {
    let (mut scheduler, _) = Scheduler::new();

    assert!(!scheduler.has_process());
    assert!(!scheduler.has_ready_process());
    assert_eq!(scheduler.next_process(), None);

    // Scheduling an unknown process should do nothing.
    scheduler.mark_ready(ProcessId(0));
    assert!(!scheduler.has_process());
    assert!(!scheduler.has_ready_process());
    assert_eq!(scheduler.next_process(), None);
}

/* NOTE: old test that tests Scheduler::run_process.
async fn pending_actor(_ctx: actor::LocalContext<!>) -> Result<(), !> {
    pending!();
    Ok(())
}

#[test]
fn running_process() {
    let (mut scheduler, mut scheduler_ref) = Scheduler::new();
    let mut runtime_ref = test::runtime();

    // Shouldn't run any process yet, since none are added.
    assert!(!scheduler.has_process());
    assert!(!scheduler.has_ready_process());
    assert!(!scheduler.run_process(&mut runtime_ref));

    // Add an actor to the scheduler.
    let process_entry = scheduler_ref.add_process();
    assert_eq!(process_entry.pid(), ProcessId(0));
    #[allow(trivial_casts)]
    let new_actor = pending_actor as fn(_) -> _;
    let (actor, inbox, inbox_ref) = init_actor_inbox(new_actor, ()).unwrap();
    process_entry.add_actor(Priority::NORMAL, NoSupervisor, new_actor, actor, inbox, inbox_ref);

    // Newly added processes aren't ready by default.
    assert!(!!scheduler.has_process());
    assert!(!scheduler.has_ready_process());
    assert!(!scheduler.run_process(&mut runtime_ref));

    // After scheduling the process should be ready to run.
    scheduler.mark_ready(ProcessId(0));
    assert!(!!scheduler.has_process());
    assert!(scheduler.has_ready_process());
    assert!(scheduler.run_process(&mut runtime_ref));

    // Process should return pending, making it inactive.
    assert!(!!scheduler.has_process());
    assert!(!scheduler.has_ready_process());
    assert!(!scheduler.run_process(&mut runtime_ref));

    scheduler.mark_ready(ProcessId(0));
    assert!(scheduler.run_process(&mut runtime_ref));
    assert!(!scheduler.has_process());
    assert!(!scheduler.has_ready_process());
}
*/

async fn order_actor(
    _ctx: actor::LocalContext<!>,
    id: usize,
    order: Rc<RefCell<Vec<usize>>>,
) -> Result<(), !> {
    order.borrow_mut().push(id);
    Ok(())
}

#[test]
fn scheduler_run_order() {
    let (mut scheduler, _) = Scheduler::new();
    let mut runtime_ref = test::runtime();

    // The order in which the processes have been run.
    let run_order = Rc::new(RefCell::new(Vec::new()));

    // Add our processes.
    #[allow(trivial_casts)]
    let new_actor = order_actor as fn(_, _, _) -> _;
    let priorities = [Priority::LOW, Priority::NORMAL, Priority::HIGH];
    let mut pids = vec![];
    for (id, priority) in priorities.iter().enumerate() {
        let actor_entry = scheduler.add_actor();
        pids.push(actor_entry.pid());
        let (actor, inbox, inbox_ref) =
            init_actor_inbox(new_actor, (id, run_order.clone())).unwrap();
        actor_entry.add(*priority, NoSupervisor, new_actor, actor, inbox, inbox_ref);
    }

    // Schedule all processes.
    for pid in pids {
        scheduler.mark_ready(pid);
    }
    assert!(!!scheduler.has_process());
    assert!(scheduler.has_ready_process());

    // Run all processes, should be in order of priority (since there runtimes
    // are equal).
    for _ in 0..3 {
        let mut process = scheduler.next_process().unwrap();
        assert_eq!(
            process.as_mut().run(&mut runtime_ref),
            ProcessResult::Complete
        );
    }
    assert!(!scheduler.has_process());
    assert_eq!(*run_order.borrow(), vec![2usize, 1, 0]);
}

#[derive(Copy, Clone)]
struct TestAssertUnmovedNewActor;

impl NewActor for TestAssertUnmovedNewActor {
    type Message = ();
    type Argument = ();
    type Actor = AssertUnmoved<Pending<Result<(), !>>>;
    type Error = !;

    fn new(
        &mut self,
        ctx: actor::LocalContext<Self::Message>,
        _arg: Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        // In the test we need the access to the inbox, to achieve that we can't
        // drop the context, so we forget about it here leaking the inbox.
        forget(ctx);
        Ok(pending().assert_unmoved())
    }
}

#[test]
fn assert_process_unmoved() {
    let (mut scheduler, _) = Scheduler::new();
    let mut runtime_ref = test::runtime();

    let new_actor = TestAssertUnmovedNewActor;
    let (actor, inbox, inbox_ref) = init_actor_inbox(new_actor, ()).unwrap();

    let actor_entry = scheduler.add_actor();
    let pid = actor_entry.pid();
    actor_entry.add(
        Priority::NORMAL,
        NoSupervisor,
        new_actor,
        actor,
        inbox,
        inbox_ref,
    );

    // Schedule and run the process multiple times, ensure it's not moved in the
    // process.
    scheduler.mark_ready(pid);
    let mut process = scheduler.next_process().unwrap();
    assert_eq!(
        process.as_mut().run(&mut runtime_ref),
        ProcessResult::Pending
    );
    scheduler.add_process(process);

    scheduler.mark_ready(pid);
    let mut process = scheduler.next_process().unwrap();
    assert_eq!(
        process.as_mut().run(&mut runtime_ref),
        ProcessResult::Pending
    );
    scheduler.add_process(process);

    scheduler.mark_ready(pid);
    let mut process = scheduler.next_process().unwrap();
    assert_eq!(
        process.as_mut().run(&mut runtime_ref),
        ProcessResult::Pending
    );
}
