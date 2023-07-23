//! Tests for the process module.

use std::cmp::Ordering;
use std::future::{pending, Future};
use std::pin::Pin;
use std::task::{self, Poll};
use std::thread::sleep;
use std::time::Duration;

use crate::process::{FutureProcess, Process, ProcessData, ProcessId};
use crate::spawn::options::Priority;
use crate::test::{assert_size, AssertUnmoved};

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
fn size_assertions() {
    assert_size::<ProcessId>(8);
    assert_size::<Priority>(1);
    assert_size::<ProcessData<Box<dyn Process>>>(32);
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
