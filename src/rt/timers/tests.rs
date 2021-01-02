use std::collections::HashSet;
use std::time::{Duration, Instant};

use crate::rt::ProcessId;

use super::TimingWheel;

const PID1: ProcessId = ProcessId(1);

#[test]
fn add_deadline() {
    let mut timers = TimingWheel::new();

    let deadline = Instant::now();
    timers.add_deadline(PID1, deadline);
    assert_within_1ms(timers.next_deadline(), Some(deadline));
}

#[test]
fn remove_deadline() {
    let mut timers = TimingWheel::new();

    let deadline = Instant::now();
    timers.add_deadline(PID1, deadline);
    timers.remove_deadline(PID1, deadline);
    assert_eq!(timers.next_deadline(), None);
}

#[test]
fn remove_not_existing_deadline() {
    let mut timers = TimingWheel::new();

    let deadline = Instant::now();
    timers.remove_deadline(PID1, deadline);
    assert_eq!(timers.next_deadline(), None);
}

#[test]
fn next_deadline() {
    let mut timers = TimingWheel::new();
    let now = Instant::now();

    let deadline1 = now + Duration::from_millis(100);
    timers.add_deadline(PID1, deadline1);
    assert_within_1ms(timers.next_deadline(), Some(deadline1));

    // Before `deadline1`.
    let deadline2 = now + Duration::from_millis(10);
    timers.add_deadline(PID1, deadline2);
    assert_within_1ms(timers.next_deadline(), Some(deadline2));

    // After `deadline2`.
    let deadline3 = now + Duration::from_millis(200);
    timers.add_deadline(PID1, deadline3);
    assert_within_1ms(timers.next_deadline(), Some(deadline2));
}

#[test]
fn next_deadline_empty() {
    let timers = TimingWheel::new();
    assert_eq!(timers.next_deadline(), None);
}

#[test]
fn empty_timer() {
    let mut timers = TimingWheel::new();
    assert_eq!(timers.next_deadline(), None);
    let mut iter = timers.deadlines(Instant::now());
    assert_eq!(iter.next(), None);
    assert_eq!(iter.next(), None);
    assert_eq!(iter.next(), None);
    assert_eq!(iter.count(), 0);
}

#[test]
fn deadlines() {
    let mut timers = TimingWheel::new();
    let now = Instant::now();

    timers.add_deadline(PID1, now);

    let mut deadlines = timers.deadlines(now);
    assert_eq!(deadlines.next(), Some(PID1));
    assert_eq!(deadlines.next(), None);
    drop(deadlines);

    let mut deadlines = timers.deadlines(now);
    assert_eq!(deadlines.next(), None);
}

#[test]
fn many_deadlines() {
    let mut timers = TimingWheel::new();
    let now = Instant::now();

    const N: usize = 5_000;
    let mut pids = (1..=N)
        .map(|id| {
            let pid = ProcessId(id);
            let deadline = now + Duration::from_millis(id as u64);
            timers.add_deadline(pid, deadline);
            pid
        })
        .collect::<HashSet<_>>();

    let last_deadline = now + Duration::from_secs(20);
    timers.add_deadline(ProcessId(N + 10), last_deadline);

    let remove_deadline = now + Duration::from_millis(10);
    let pid = ProcessId(N + 20);
    timers.add_deadline(pid, remove_deadline);
    timers.remove_deadline(pid, remove_deadline);

    let mut deadlines = timers.deadlines(now);
    assert_eq!(deadlines.next(), None);
    drop(deadlines);

    for pid in timers.deadlines(now + Duration::from_millis((N + 2) as u64)) {
        assert!(pids.remove(&pid), "unexpected pid: {}", pid);
    }
    assert!(pids.is_empty(), "missing pids: {:?}", pids);

    assert_within_1ms(timers.next_deadline(), Some(last_deadline));
}

#[test]
fn deadlines_empty() {
    let mut timers = TimingWheel::new();

    let mut deadlines = timers.deadlines(Instant::now());
    assert_eq!(deadlines.next(), None);
}

/// Asserts that `left` is within 1 millisecond of the time in `right`.
#[track_caller]
fn assert_within_1ms(left: Option<Instant>, right: Option<Instant>) {
    match (left, right) {
        (Some(left), Some(right)) if left == right => {}
        (Some(left), Some(right))
            if left < right && right.duration_since(left).as_millis() <= 1 => {}
        (Some(left), Some(right))
            if left > right && left.duration_since(right).as_millis() <= 1 => {}
        _ => assert_eq!(left, right),
    }
}
