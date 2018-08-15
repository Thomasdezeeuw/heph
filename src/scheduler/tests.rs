//! Tests for the scheduler.

use std::cmp::Ordering;
use std::mem;
use std::thread::sleep;
use std::time::Duration;

use env_logger;

use crate::process::{Process, ProcessId, ProcessResult};
use crate::scheduler::{Priority, ProcessData, ProcessState, Scheduler};
use crate::system::ActorSystemRef;
use crate::test;
use crate::util::Shared;

fn assert_size<T>(expected: usize) {
    assert_eq!(mem::size_of::<T>(), expected);
}

#[test]
fn size_assertions() {
    assert_size::<ProcessState>(mem::size_of::<Box<ProcessData>>());
}

#[derive(Debug)]
pub struct EmptyProcess;

impl Process for EmptyProcess {
    fn run(&mut self, _: &mut ActorSystemRef) -> ProcessResult {
        unreachable!();
    }
}

#[test]
fn process_data_equality() {
    let pid = ProcessId(0);
    let left = ProcessData::new(pid, Priority::LOW, EmptyProcess);
    let right = ProcessData::new(pid, Priority::HIGH, EmptyProcess);
    // Equality solely based on the process id.
    assert_eq!(left, right);
}

#[test]
fn process_data_order() {
    let pid = ProcessId(0);

    // Same fair_runtime and priority.
    let mut left = ProcessData::new(pid, Priority::NORMAL, EmptyProcess);
    let right = ProcessData::new(pid, Priority::NORMAL, EmptyProcess);
    assert_eq!(left.cmp(&right), Ordering::Equal);

    // Different fair_runtime
    left.fair_runtime += Duration::from_millis(1);
    assert_eq!(left.cmp(&right), Ordering::Less);
    assert_eq!(right.cmp(&left), Ordering::Greater);

    // Same fair_runtime, left has a higher priority.
    let left = ProcessData::new(pid, Priority::HIGH, EmptyProcess);
    let right = ProcessData::new(pid, Priority::NORMAL, EmptyProcess);
    assert_eq!(left.cmp(&right), Ordering::Greater);

    // Different fair_runtime, left has a higher priority.
    let mut left = ProcessData::new(pid, Priority::HIGH, EmptyProcess);
    left.fair_runtime += Duration::from_millis(1);
    let right = ProcessData::new(pid, Priority::NORMAL, EmptyProcess);
    assert_eq!(left.cmp(&right), Ordering::Less);
}

#[derive(Debug)]
struct SleepyProcess(Duration);

impl Process for SleepyProcess {
    fn run(&mut self, _: &mut ActorSystemRef) -> ProcessResult {
        sleep(self.0);
        ProcessResult::Pending
    }
}

#[test]
fn process_data() {
    const SLEEP_MARGIN: Duration = Duration::from_millis(5);

    let sleep = Duration::from_millis(1);
    let pid = ProcessId(0);

    let mut system_ref = test::system_ref();

    let priorities = [Priority::LOW, Priority::NORMAL, Priority::HIGH];
    for priority in priorities.into_iter() {
        let priority = *priority;
        let mut process_data = ProcessData::new(pid, priority, SleepyProcess(sleep));

        assert_eq!(process_data.id(), pid);

        assert_eq!(process_data.run(&mut system_ref), ProcessResult::Pending);
        // Roughly check that the fair runtime is set correctly.
        assert!(process_data.fair_runtime > sleep * priority);
        assert!(process_data.fair_runtime < sleep * priority + (SLEEP_MARGIN * priority));
    }
}

#[test]
fn process_state() {
    let process_data = ProcessData::new(ProcessId(0), Priority::NORMAL, EmptyProcess);

    let mut process_state = ProcessState::Active;
    assert!(process_state.is_active());

    process_state.mark_inactive(Box::new(process_data));
    assert!(!process_state.is_active());

    let process_data = process_state.mark_active();
    assert!(process_state.is_active());
    assert_eq!(process_data.id(), ProcessId(0));
}

#[derive(Debug)]
struct TestProcess {
    id: ProcessId,
    order: Shared<Vec<usize>>,
}

impl Process for TestProcess {
    fn run(&mut self, _: &mut ActorSystemRef) -> ProcessResult {
        self.order.borrow_mut().push(self.id.0);
        match self.id.0 {
            0 => {
                self.id = ProcessId(10);
                ProcessResult::Pending
            },
            _ => ProcessResult::Complete,
        }
    }
}

#[test]
fn scheduler() {
    env_logger::init();

    let mut system_ref = test::system_ref();

    // In which order the processes have been run.
    let mut run_order = Shared::new(Vec::new());

    let (mut scheduler, mut scheduler_ref) = Scheduler::new();

    // Add our processes.
    let priorities = [Priority::LOW, Priority::NORMAL, Priority::HIGH];
    for priority in priorities.iter() {
        let priority = *priority;
        let process_entry = scheduler_ref.add_process();
        let id = process_entry.id();
        let process = TestProcess { id, order: run_order.clone() };
        process_entry.add(process, priority);
    }

    // Schedule and run all processes.
    for id in 0 .. 3 {
        scheduler.schedule(ProcessId(id));
    }
    assert!(scheduler.process_ready());
    for _ in 0 .. 3 {
        let process_ran = scheduler.run_process(&mut system_ref);
        assert!(process_ran);
    }

    // Only a single process left, which is inactive.
    assert_eq!(scheduler.run_process(&mut system_ref), false);
    assert!(!scheduler.process_ready());

    // Active and run the last process, which also completes.
    scheduler.schedule(ProcessId(0));
    assert!(scheduler.process_ready());
    assert_eq!(scheduler.run_process(&mut system_ref), true);
    assert_eq!(scheduler.run_process(&mut system_ref), false);
    assert!(!scheduler.process_ready());

    assert_eq!(*run_order.borrow_mut(), vec![2, 1, 0, 10]);
}
