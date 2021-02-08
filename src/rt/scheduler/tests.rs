//! Tests for the scheduler.

use std::cmp::Ordering;
use std::future::{pending, Pending};
use std::marker::PhantomData;
use std::mem::{self, forget};
use std::pin::Pin;
use std::thread::sleep;
use std::time::Duration;

use crate::actor::{self, NewActor};
use crate::rt::process::{Process, ProcessId, ProcessResult};
use crate::rt::scheduler::{local, shared, Priority, ProcessData};
use crate::rt::RuntimeRef;
use crate::test::{self, AssertUnmoved};

fn assert_size<T>(expected: usize) {
    assert_eq!(mem::size_of::<T>(), expected);
}

#[test]
fn size_assertions() {
    assert_size::<ProcessId>(8);
    assert_size::<Priority>(1);
    assert_size::<local::ProcessData>(40);
    assert_size::<shared::ProcessData>(40);
}

#[test]
#[allow(clippy::eq_op)] // Need to compare `ProcessData` to itself.
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

struct TestAssertUnmovedNewActor<C>(PhantomData<C>);

impl<C> Clone for TestAssertUnmovedNewActor<C> {
    fn clone(&self) -> TestAssertUnmovedNewActor<C> {
        TestAssertUnmovedNewActor(PhantomData)
    }
}

impl<C> Copy for TestAssertUnmovedNewActor<C> {}

impl<C> NewActor for TestAssertUnmovedNewActor<C> {
    type Message = ();
    type Argument = ();
    type Actor = AssertUnmoved<Pending<Result<(), !>>>;
    type Error = !;
    type Context = C;

    fn new(
        &mut self,
        ctx: actor::Context<Self::Message, Self::Context>,
        _: Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        // In the test we need the access to the inbox, to achieve that we can't
        // drop the context, so we forget about it here leaking the inbox.
        forget(ctx);
        Ok(AssertUnmoved::new(pending()))
    }
}

mod local_scheduler {
    use std::cell::RefCell;
    use std::marker::PhantomData;
    use std::pin::Pin;
    use std::rc::Rc;
    use std::time::Duration;

    use crate::actor;
    use crate::rt::process::{Process, ProcessId, ProcessResult};
    use crate::rt::scheduler::{LocalScheduler, Priority, ProcessData};
    use crate::supervisor::NoSupervisor;
    use crate::test::{self, init_local_actor_with_inbox};

    use super::{NopTestProcess, TestAssertUnmovedNewActor};

    async fn simple_actor(_: actor::Context<!>) -> Result<(), !> {
        Ok(())
    }

    #[test]
    fn has_process() {
        let mut scheduler = LocalScheduler::new();
        assert!(!scheduler.has_process());
        assert!(!scheduler.has_ready_process());

        let process: Pin<Box<ProcessData<dyn Process>>> = Box::pin(ProcessData {
            priority: Priority::default(),
            fair_runtime: Duration::from_secs(0),
            process: Box::pin(NopTestProcess),
        });
        scheduler.add_process(process);
        assert!(scheduler.has_process());
        assert!(!scheduler.has_ready_process());
    }

    #[test]
    fn add_actor() {
        let mut scheduler = LocalScheduler::new();

        let actor_entry = scheduler.add_actor();
        let new_actor = simple_actor as fn(_) -> _;
        let (actor, inbox, _) = init_local_actor_with_inbox(new_actor, ()).unwrap();
        actor_entry.add(
            Priority::NORMAL,
            NoSupervisor,
            new_actor,
            actor,
            inbox,
            false,
        );
        assert!(scheduler.has_process());
        assert!(!scheduler.has_ready_process());
    }

    #[test]
    fn mark_ready() {
        let mut scheduler = LocalScheduler::new();

        // Incorrect (outdated) pid should be ok.
        scheduler.mark_ready(ProcessId(1));

        let actor_entry = scheduler.add_actor();
        let pid = actor_entry.pid();
        let new_actor = simple_actor as fn(_) -> _;
        let (actor, inbox, _) = init_local_actor_with_inbox(new_actor, ()).unwrap();
        actor_entry.add(
            Priority::NORMAL,
            NoSupervisor,
            new_actor,
            actor,
            inbox,
            false,
        );

        scheduler.mark_ready(pid);
        assert!(scheduler.has_process());
        assert!(scheduler.has_ready_process());
    }

    #[test]
    fn next_process() {
        let mut scheduler = LocalScheduler::new();

        let actor_entry = scheduler.add_actor();
        let pid = actor_entry.pid();
        let new_actor = simple_actor as fn(_) -> _;
        let (actor, inbox, _) = init_local_actor_with_inbox(new_actor, ()).unwrap();
        actor_entry.add(
            Priority::NORMAL,
            NoSupervisor,
            new_actor,
            actor,
            inbox,
            false,
        );
        scheduler.mark_ready(pid);

        if let Some(process) = scheduler.next_process() {
            assert_eq!(process.as_ref().id(), pid);
            assert!(!scheduler.has_process());
            assert!(!scheduler.has_ready_process());
        } else {
            panic!("expected a process");
        }
    }

    #[test]
    fn next_process_order() {
        let mut scheduler = LocalScheduler::new();

        let new_actor = simple_actor as fn(_) -> _;
        // Actor 1.
        let actor_entry = scheduler.add_actor();
        let pid1 = actor_entry.pid();
        let (actor, inbox, _) = init_local_actor_with_inbox(new_actor, ()).unwrap();
        actor_entry.add(Priority::LOW, NoSupervisor, new_actor, actor, inbox, true);
        // Actor 2.
        let actor_entry = scheduler.add_actor();
        let pid2 = actor_entry.pid();
        let (actor, inbox, _) = init_local_actor_with_inbox(new_actor, ()).unwrap();
        actor_entry.add(Priority::HIGH, NoSupervisor, new_actor, actor, inbox, true);
        // Actor 3.
        let actor_entry = scheduler.add_actor();
        let pid3 = actor_entry.pid();
        let (actor, inbox, _) = init_local_actor_with_inbox(new_actor, ()).unwrap();
        actor_entry.add(
            Priority::NORMAL,
            NoSupervisor,
            new_actor,
            actor,
            inbox,
            true,
        );

        assert!(scheduler.has_process());
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
        let mut scheduler = LocalScheduler::new();

        let actor_entry = scheduler.add_actor();
        let pid = actor_entry.pid();
        let new_actor = simple_actor as fn(_) -> _;
        let (actor, inbox, _) = init_local_actor_with_inbox(new_actor, ()).unwrap();
        actor_entry.add(
            Priority::NORMAL,
            NoSupervisor,
            new_actor,
            actor,
            inbox,
            false,
        );

        assert!(scheduler.next_process().is_none());
        assert!(scheduler.has_process());
        assert!(!scheduler.has_ready_process());

        scheduler.mark_ready(pid);
        assert!(scheduler.has_process());
        assert!(scheduler.has_ready_process());
        let process = scheduler.next_process().unwrap();
        assert_eq!(process.as_ref().id(), pid);
    }

    #[test]
    fn add_process_marked_ready() {
        let mut scheduler = LocalScheduler::new();

        let actor_entry = scheduler.add_actor();
        let pid = actor_entry.pid();
        let new_actor = simple_actor as fn(_) -> _;
        let (actor, inbox, _) = init_local_actor_with_inbox(new_actor, ()).unwrap();
        actor_entry.add(
            Priority::NORMAL,
            NoSupervisor,
            new_actor,
            actor,
            inbox,
            true,
        );

        let process = scheduler.next_process().unwrap();
        scheduler.add_process(process);
        assert!(scheduler.has_process());
        assert!(!scheduler.has_ready_process());

        scheduler.mark_ready(pid);
        assert!(scheduler.has_process());
        assert!(scheduler.has_ready_process());
        let process = scheduler.next_process().unwrap();
        assert_eq!(process.as_ref().id(), pid);
    }

    #[test]
    fn scheduler_run_order() {
        let mut scheduler = LocalScheduler::new();
        let mut runtime_ref = test::runtime();

        // The order in which the processes have been run.
        let run_order = Rc::new(RefCell::new(Vec::new()));

        async fn order_actor(
            _: actor::Context<!>,
            id: usize,
            order: Rc<RefCell<Vec<usize>>>,
        ) -> Result<(), !> {
            order.borrow_mut().push(id);
            Ok(())
        }

        // Add our processes.
        let new_actor = order_actor as fn(_, _, _) -> _;
        let priorities = [Priority::LOW, Priority::NORMAL, Priority::HIGH];
        let mut pids = vec![];
        for (id, priority) in priorities.iter().enumerate() {
            let actor_entry = scheduler.add_actor();
            pids.push(actor_entry.pid());
            let (actor, inbox, _) =
                init_local_actor_with_inbox(new_actor, (id, run_order.clone())).unwrap();
            actor_entry.add(*priority, NoSupervisor, new_actor, actor, inbox, true);
        }

        assert!(scheduler.has_process());
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

    #[test]
    fn assert_process_unmoved() {
        let mut scheduler = LocalScheduler::new();
        let mut runtime_ref = test::runtime();

        let new_actor = TestAssertUnmovedNewActor(PhantomData);
        let (actor, inbox, _) = init_local_actor_with_inbox(new_actor, ()).unwrap();

        let actor_entry = scheduler.add_actor();
        let pid = actor_entry.pid();
        actor_entry.add(
            Priority::NORMAL,
            NoSupervisor,
            new_actor,
            actor,
            inbox,
            true,
        );

        // Run the process multiple times, ensure it's not moved in the process.
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
}

mod shared_scheduler {
    use std::marker::PhantomData;
    use std::sync::{Arc, Mutex};

    use crate::actor::{self, context};
    use crate::rt::process::ProcessId;
    use crate::rt::process::ProcessResult;
    use crate::rt::scheduler::{Priority, Scheduler};
    use crate::supervisor::NoSupervisor;
    use crate::test::{self, init_actor_with_inbox};

    use super::TestAssertUnmovedNewActor;

    async fn simple_actor(_: actor::Context<!, context::ThreadSafe>) -> Result<(), !> {
        Ok(())
    }

    #[test]
    fn is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<Scheduler>();
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
            let (actor, inbox, _) =
                init_actor_with_inbox(new_actor, (id, run_order.clone())).unwrap();
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

    #[test]
    fn assert_process_unmoved() {
        let scheduler = Scheduler::new();
        let mut runtime_ref = test::runtime();

        let new_actor = TestAssertUnmovedNewActor(PhantomData);
        let (actor, inbox, _) = init_actor_with_inbox(new_actor, ()).unwrap();

        let actor_entry = scheduler.add_actor();
        let pid = actor_entry.pid();
        actor_entry.add(
            Priority::NORMAL,
            NoSupervisor,
            new_actor,
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
}
