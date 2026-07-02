use std::cmp::Ordering;
use std::task::Poll;
use std::time::{Duration, Instant};

use heph_rt::setup::scheduler::{Cfs, RunStats, Schedule};
use heph_rt::spawn::options::Priority;

#[test]
#[rustfmt::skip]
fn cfs() {
    let mut process_high = Cfs::new(Priority::HIGH);
    let mut process_normal = Cfs::new(Priority::NORMAL);
    let mut process_low = Cfs::new(Priority::LOW);

    // If the `fair_runtime` is equal we only compare based on the priority.
    assert_eq!(Schedule::order(&process_high, &process_high), Ordering::Equal);
    assert_eq!(Schedule::order(&process_high, &process_normal), Ordering::Greater);
    assert_eq!(Schedule::order(&process_high, &process_low), Ordering::Greater);

    assert_eq!(Schedule::order(&process_normal, &process_high), Ordering::Less);
    assert_eq!(Schedule::order(&process_normal, &process_normal), Ordering::Equal);
    assert_eq!(Schedule::order(&process_normal, &process_low), Ordering::Greater);

    assert_eq!(Schedule::order(&process_low, &process_high), Ordering::Less);
    assert_eq!(Schedule::order(&process_low, &process_normal), Ordering::Less);
    assert_eq!(Schedule::order(&process_low, &process_low), Ordering::Equal);

    // Update the fair runtime.
    let start = Instant::now();
    let stats = RunStats::new(start, start + Duration::from_millis(5), Poll::Pending);
    process_high.update(&stats);
    assert_eq!(process_high.fair_runtime(), Duration::from_millis(25));

    let stats = RunStats::new(start, start + Duration::from_millis(2), Poll::Pending);
    process_normal.update(&stats);
    assert_eq!(process_normal.fair_runtime(), Duration::from_millis(20));

    let stats = RunStats::new(start, start + Duration::from_millis(1), Poll::Pending);
    process_low.update(&stats);
    assert_eq!(process_low.fair_runtime(), Duration::from_millis(15));

    // If the `fair_runtime` is not equal we order by that.
    assert_eq!(Schedule::order(&process_high, &process_high), Ordering::Equal);
    assert_eq!(Schedule::order(&process_high, &process_normal), Ordering::Less);
    assert_eq!(Schedule::order(&process_high, &process_low), Ordering::Less);

    assert_eq!(Schedule::order(&process_normal, &process_high), Ordering::Greater);
    assert_eq!(Schedule::order(&process_normal, &process_normal), Ordering::Equal);
    assert_eq!(Schedule::order(&process_normal, &process_low), Ordering::Less);

    assert_eq!(Schedule::order(&process_low, &process_high), Ordering::Greater);
    assert_eq!(Schedule::order(&process_low, &process_normal), Ordering::Greater);
    assert_eq!(Schedule::order(&process_low, &process_low), Ordering::Equal);
}
