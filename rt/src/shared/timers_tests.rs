use std::time::Duration;

use crate::process::ProcessId;

use super::{Timers, DURATION_PER_SLOT, NS_PER_SLOT, SLOTS};

const PID: ProcessId = ProcessId(100);
const PID2: ProcessId = ProcessId(200);

#[test]
fn add_deadline_first_slot() {
    let timers = Timers::new();
    let epoch = timers.epoch.read().unwrap().time;
    let deadline = epoch + Duration::from_millis(100);
    timers.add(PID, deadline);
    assert_eq!(timers.next(), Some(deadline));
    assert_eq!(timers.remove_next(epoch), None);
    assert_eq!(timers.remove_next(deadline), Some(PID));
    assert_eq!(timers.remove_next(deadline), None);
}

#[test]
fn add_deadline_second_slot() {
    let timers = Timers::new();
    let epoch = timers.epoch.read().unwrap().time;
    let deadline = epoch + Duration::from_nanos(NS_PER_SLOT as u64 + 100);
    timers.add(PID, deadline);
    assert_eq!(timers.next(), Some(deadline));
    assert_eq!(timers.remove_next(epoch), None);
    assert_eq!(timers.epoch.read().unwrap().index, 0);
    assert_eq!(timers.next(), Some(deadline));
    assert_eq!(timers.remove_next(deadline), Some(PID));
    assert_eq!(timers.epoch.read().unwrap().index, 1);
    assert_eq!(timers.remove_next(deadline), None);
}

#[test]
fn add_deadline_overflow() {
    let timers = Timers::new();
    let epoch = timers.epoch.read().unwrap().time;
    let deadline = epoch + Duration::from_nanos(SLOTS as u64 * NS_PER_SLOT as u64 + 10);
    timers.add(PID, deadline);
    assert_eq!(timers.next(), Some(deadline));
    assert_eq!(timers.remove_next(epoch), None);
    assert_eq!(timers.epoch.read().unwrap().index, 0);
    assert_eq!(timers.next(), Some(deadline));
    assert_eq!(timers.remove_next(deadline), Some(PID));
    // Should have advanced the epoch to come back around to 0.
    assert_eq!(timers.epoch.read().unwrap().index, 0);
    assert_eq!(timers.remove_next(deadline), None);
}

#[test]
fn add_deadline_to_all_slots() {
    let timers = Timers::new();
    let epoch = timers.epoch.read().unwrap().time;

    // Add a deadline to all slots and the overflow list.
    for n in 0..=SLOTS {
        let deadline = epoch + Duration::from_nanos((n as u64 * NS_PER_SLOT as u64) + 10);
        timers.add(ProcessId(n), deadline);
    }

    let first_deadline = epoch + Duration::from_nanos(10);
    assert_eq!(timers.next(), Some(first_deadline));
    assert_eq!(timers.remove_next(epoch), None);
    assert_eq!(timers.epoch.read().unwrap().index, 0);

    let mut expected_next_deadline = first_deadline;
    let mut expected_index = 0;
    for n in 0..=SLOTS {
        assert_eq!(timers.next(), Some(expected_next_deadline));
        let now = expected_next_deadline + Duration::from_nanos(1);
        assert_eq!(timers.remove_next(now), Some(ProcessId(n)));
        assert_eq!(timers.epoch.read().unwrap().index, expected_index);
        assert_eq!(timers.remove_next(now), None);
        assert_eq!(timers.epoch.read().unwrap().index, expected_index);

        expected_index = (expected_index + 1) % SLOTS as u8;
        expected_next_deadline += DURATION_PER_SLOT;
    }
}

#[test]
fn add_deadline_in_the_past() {
    let timers = Timers::new();
    let epoch = timers.epoch.read().unwrap().time;
    timers.add(PID, epoch - Duration::from_secs(1));
    assert_eq!(timers.next(), Some(epoch));
    assert_eq!(timers.remove_next(epoch), Some(PID));
}

#[test]
fn adding_earlier_deadline() {
    let timers = Timers::new();
    let epoch = timers.epoch.read().unwrap().time;
    let deadline1 = epoch + Duration::from_secs(2);
    let deadline2 = epoch + Duration::from_secs(1);
    timers.add(PID, deadline1);
    timers.add(PID2, deadline2);
    assert_eq!(timers.next(), Some(deadline2));
    assert_eq!(timers.remove_next(deadline1), Some(PID2));
    assert_eq!(timers.remove_next(deadline1), Some(PID));
    assert_eq!(timers.remove_next(deadline1), None);
}

#[test]
fn remove_deadline() {
    let timers = Timers::new();
    let epoch = timers.epoch.read().unwrap().time;
    let deadline = epoch + Duration::from_millis(10);
    timers.add(PID, deadline);
    timers.remove(PID, deadline);
    assert_eq!(timers.next(), None);
    assert_eq!(timers.remove_next(deadline), None);
}

#[test]
fn remove_never_added_deadline() {
    let timers = Timers::new();
    let epoch = timers.epoch.read().unwrap().time;
    let deadline = epoch + Duration::from_millis(10);
    assert_eq!(timers.next(), None);
    assert_eq!(timers.remove_next(deadline), None);
    timers.remove(PID, deadline);
    assert_eq!(timers.next(), None);
    assert_eq!(timers.remove_next(deadline), None);
}

#[test]
fn remove_expired_deadline() {
    let timers = Timers::new();
    let epoch = timers.epoch.read().unwrap().time;
    let deadline = epoch + Duration::from_millis(10);
    timers.add(PID, deadline);
    assert_eq!(timers.next(), Some(deadline));
    assert_eq!(timers.remove_next(deadline), Some(PID));
    assert_eq!(timers.remove_next(deadline), None);
    timers.remove(PID, deadline);
    assert_eq!(timers.next(), None);
    assert_eq!(timers.remove_next(deadline), None);
}

#[test]
fn remove_deadline_from_all_slots() {
    let timers = Timers::new();
    let epoch = timers.epoch.read().unwrap().time;

    // Add a deadline to all slots and the overflow list.
    for n in 0..=SLOTS {
        let deadline = epoch + Duration::from_nanos((n as u64 * NS_PER_SLOT as u64) + 10);
        timers.add(ProcessId(n), deadline);
    }

    let first_deadline = epoch + Duration::from_nanos(10);
    assert_eq!(timers.next(), Some(first_deadline));
    assert_eq!(timers.remove_next(epoch), None);
    assert_eq!(timers.epoch.read().unwrap().index, 0);

    let mut next_deadline = first_deadline;
    for n in 0..=SLOTS {
        timers.remove(ProcessId(n), next_deadline);
        next_deadline += DURATION_PER_SLOT;

        if n == SLOTS {
            assert_eq!(timers.next(), None);
        } else {
            assert_eq!(timers.next(), Some(next_deadline));
        }
    }
}

#[test]
fn remove_deadline_from_all_slots_interleaved() {
    let timers = Timers::new();
    let epoch = timers.epoch.read().unwrap().time;

    // Add a deadline to all slots and the overflow list.
    for n in 0..=SLOTS {
        let deadline = epoch + Duration::from_nanos((n as u64 * NS_PER_SLOT as u64) + 10);
        timers.add(ProcessId(n), deadline);
        timers.remove(ProcessId(n), deadline);
    }

    assert_eq!(timers.next(), None);
    assert_eq!(timers.remove_next(epoch), None);
    assert_eq!(timers.epoch.read().unwrap().index, 0);
}

#[test]
fn remove_deadline_after_epoch_advance() {
    let timers = Timers::new();
    let epoch = timers.epoch.read().unwrap().time;

    // Add a deadline to all slots and the overflow list.
    for n in 0..=SLOTS {
        let deadline = epoch + Duration::from_nanos((n as u64 * NS_PER_SLOT as u64) + 10);
        timers.add(ProcessId(n), deadline);
    }

    let first_deadline = epoch + Duration::from_nanos(10);
    let now = epoch + DURATION_PER_SLOT;
    assert_eq!(timers.next(), Some(first_deadline));
    assert_eq!(timers.remove_next(now), Some(ProcessId(0)));
    assert_eq!(timers.remove_next(now), None);
    assert_eq!(timers.epoch.read().unwrap().index, 1);
    assert_eq!(timers.next(), Some(first_deadline + DURATION_PER_SLOT));

    let mut next_deadline = first_deadline + DURATION_PER_SLOT;
    for n in 1..=SLOTS {
        timers.remove(ProcessId(n), next_deadline);
        next_deadline += DURATION_PER_SLOT;

        if n == SLOTS {
            assert_eq!(timers.next(), None);
        } else {
            assert_eq!(timers.next(), Some(next_deadline));
        }
    }
}

#[test]
fn remove_deadline_in_the_past() {
    let timers = Timers::new();
    let epoch = timers.epoch.read().unwrap().time;
    let deadline = epoch - Duration::from_secs(1);
    timers.add(PID, deadline);
    assert_eq!(timers.next(), Some(epoch));
    timers.remove(PID, deadline);
    assert_eq!(timers.next(), None);
    assert_eq!(timers.remove_next(epoch), None);
}
