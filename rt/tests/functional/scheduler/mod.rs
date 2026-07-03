use std::future::pending;
use std::pin::Pin;
use std::task::{self, Poll};

use heph_rt::setup::scheduler::{FutureTask, Process, Scheduler, Task};
use heph_rt::spawn::options::Priority;

use crate::util::AssertUnmoved;

mod cfs;
mod local_scheduler;
mod process_id;

#[test]
fn future_task() {
    struct MyTask;

    impl Future for MyTask {
        type Output = ();

        fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<()> {
            unimplemented!();
        }
    }

    async fn my_task() {
        unimplemented!();
    }

    assert_eq!(FutureTask::new(MyTask).name(), "MyTask");
    assert_eq!(FutureTask::new(std::future::pending()).name(), "Pending");
    assert_eq!(FutureTask::new(my_task()).name(), "my_task");
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
        Poll::Pending
    }
}

fn test_scheduler<S: Scheduler>(mut scheduler: S) {
    // Start in an empty state.
    assert!(scheduler.next_process().is_none());
    assert!(!scheduler.has_process());
    assert!(!scheduler.has_ready_process());
    assert_eq!(scheduler.processes_ready(), 0);
    assert_eq!(scheduler.processes_inactive(), 0);

    // Adding a new task should mark it as ready.
    let expected_pid = scheduler.add_task(Priority::NORMAL, NopTestTask);
    assert!(scheduler.has_process());
    assert!(scheduler.has_ready_process());
    assert_eq!(scheduler.processes_ready(), 1);
    assert_eq!(scheduler.processes_inactive(), 0);

    // Remove the task again.
    let (mut process, waker) = scheduler.next_process().unwrap();
    assert_eq!(process.id(), expected_pid);
    assert_eq!(process.name(), "NopTestTask");
    assert!(scheduler.next_process().is_none());
    // NOTE: not checking as `process` is still part of the scheduler, so it's
    // allowed to return true or false here.
    //assert!(scheduler.has_process());
    assert!(!scheduler.has_ready_process());
    assert_eq!(scheduler.processes_ready(), 0);
    // NOTE: not checking inactive processes here either, as it can be 0 or 1.
    //assert_eq!(scheduler.processes_inactive(), 0);

    // Run the process and add it back, should mark it as inactive.
    let mut ctx = task::Context::from_waker(&waker);
    let stats = Pin::new(&mut process).run(&mut ctx);
    scheduler.add_back_process(process, stats);
    drop(ctx);
    assert!(scheduler.next_process().is_none());
    assert!(scheduler.has_process());
    assert!(!scheduler.has_ready_process());
    assert_eq!(scheduler.processes_ready(), 0);
    assert_eq!(scheduler.processes_inactive(), 1);

    // Wake the process.
    waker.wake();
    scheduler.schedule_processes();
    assert!(scheduler.has_process());
    assert!(scheduler.has_ready_process());
    assert_eq!(scheduler.processes_ready(), 1);
    assert_eq!(scheduler.processes_inactive(), 0);

    // Mark the process as complete.
    let (process, _) = scheduler.next_process().unwrap();
    assert_eq!(process.id(), expected_pid);
    assert_eq!(process.name(), "NopTestTask");
    scheduler.complete_process(process).unwrap();
    assert!(scheduler.next_process().is_none());
    assert!(!scheduler.has_process());
    assert!(!scheduler.has_ready_process());
    assert_eq!(scheduler.processes_ready(), 0);
    assert_eq!(scheduler.processes_inactive(), 0);
}

fn test_scheduler_task_unmoved<S: Scheduler>(mut scheduler: S) {
    let task = FutureTask::new(AssertUnmoved::new(pending()));
    let _ = scheduler.add_task(Priority::NORMAL, task);

    let (mut process, waker) = scheduler.next_process().unwrap();
    let mut ctx = task::Context::from_waker(&waker);

    // All we do is poll and run it a couple of times, it should panic if the
    // actor is moved.
    let _ = Pin::new(&mut process).poll(&mut ctx);
    let _ = Pin::new(&mut process).poll(&mut ctx);
    let _ = Pin::new(&mut process).run(&mut ctx);
    let _ = Pin::new(&mut process).run(&mut ctx);
}

fn test_scheduler_wake_while_running<S: Scheduler>(mut scheduler: S) {
    let task = FutureTask::new(pending());
    let _ = scheduler.add_task(Priority::NORMAL, task);

    let (mut process, waker) = scheduler.next_process().unwrap();
    let mut ctx = task::Context::from_waker(&waker);
    let stats = Pin::new(&mut process).run(&mut ctx);
    drop(ctx);

    // Waking while the process is "running"
    waker.wake();

    // After adding the process back it's not ready, even though it got awoken
    // above.
    scheduler.add_back_process(process, stats);
    scheduler.schedule_processes();
    assert!(scheduler.has_process());
    assert!(scheduler.has_ready_process());
    assert_eq!(scheduler.processes_ready(), 1);
    assert_eq!(scheduler.processes_inactive(), 0);
}

fn test_scheduler_wake_after_completion<S: Scheduler>(mut scheduler: S) {
    let task = FutureTask::new(pending());
    let _ = scheduler.add_task(Priority::NORMAL, task);

    let (process, waker) = scheduler.next_process().unwrap();
    scheduler.complete_process(process).unwrap();

    // Waking after the process is completed shouldn't be an issue.
    waker.wake();
    scheduler.schedule_processes();
    assert!(!scheduler.has_process());
    assert!(!scheduler.has_ready_process());
    assert_eq!(scheduler.processes_ready(), 0);
    assert_eq!(scheduler.processes_inactive(), 0);
}
