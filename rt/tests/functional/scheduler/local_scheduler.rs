use std::future::pending;

use heph_rt::setup::scheduler::{Cfs, FutureTask, LocalScheduler, Process, Scheduler};
use heph_rt::spawn::options::Priority;

#[test]
fn smoke() {
    super::test_scheduler(test_scheduler());
}

#[test]
fn task_unmoved() {
    super::test_scheduler_task_unmoved(test_scheduler());
}

#[test]
fn wake_while_running() {
    super::test_scheduler_wake_while_running(test_scheduler());
}

#[test]
fn wake_after_completion() {
    super::test_scheduler_wake_after_completion(test_scheduler());
}

#[test]
fn next_process_order() {
    let mut scheduler = test_scheduler();

    let pid_low = scheduler.add_task(Priority::LOW, FutureTask::new(pending()));
    let pid_norm = scheduler.add_task(Priority::NORMAL, FutureTask::new(pending()));
    let pid_high = scheduler.add_task(Priority::HIGH, FutureTask::new(pending()));

    assert!(scheduler.has_process());
    assert!(scheduler.has_ready_process());

    // High.
    let (process1, _) = scheduler.next_process().unwrap();
    assert_eq!(process1.id(), pid_high);
    scheduler.complete_process(process1).unwrap();
    // Normal.
    let (process2, _) = scheduler.next_process().unwrap();
    assert_eq!(process2.id(), pid_norm);
    scheduler.complete_process(process2).unwrap();
    // Low.
    let (process3, _) = scheduler.next_process().unwrap();
    assert_eq!(process3.id(), pid_low);
    scheduler.complete_process(process3).unwrap();

    assert!(scheduler.next_process().is_none());
}

fn test_scheduler() -> LocalScheduler<Cfs> {
    LocalScheduler::new_testing()
}
