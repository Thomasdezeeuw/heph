use std::time::Duration;

use crate::process::ProcessId;

use super::{Timers, DURATION_PER_SLOT, NS_PER_SLOT, OVERFLOW_DURATION, SLOTS};

const PID: ProcessId = ProcessId(100);
const PID2: ProcessId = ProcessId(200);

#[test]
fn add_deadline_first_slot() {
    let mut timers = Timers::new();
    let deadline = timers.epoch + Duration::from_millis(100);
    timers.add(PID, deadline);
    assert_eq!(timers.next(), Some(deadline));
    assert_eq!(timers.remove_next(timers.epoch), None);
    assert_eq!(timers.remove_next(deadline), Some(PID));
    assert_eq!(timers.remove_next(deadline), None);
}

#[test]
fn add_deadline_second_slot() {
    let mut timers = Timers::new();
    let deadline = timers.epoch + Duration::from_nanos(NS_PER_SLOT as u64 + 10);
    timers.add(PID, deadline);
    assert_eq!(timers.next(), Some(deadline));
    assert_eq!(timers.remove_next(timers.epoch), None);
    assert_eq!(timers.index, 0);
    assert_eq!(timers.next(), Some(deadline));
    assert_eq!(timers.remove_next(deadline), Some(PID));
    assert_eq!(timers.index, 1);
    assert_eq!(timers.remove_next(deadline), None);
}

#[test]
fn add_deadline_overflow() {
    let mut timers = Timers::new();
    let deadline = timers.epoch + Duration::from_nanos(SLOTS as u64 * NS_PER_SLOT as u64 + 10);
    timers.add(PID, deadline);
    assert_eq!(timers.next(), Some(deadline));
    assert_eq!(timers.remove_next(timers.epoch), None);
    assert_eq!(timers.index, 0);
    assert_eq!(timers.next(), Some(deadline));
    assert_eq!(timers.remove_next(deadline), Some(PID));
    // Should have advanced the epoch to come back around to 0.
    assert_eq!(timers.index, 0);
    assert_eq!(timers.remove_next(deadline), None);
}

#[test]
fn add_deadline_to_all_slots() {
    let mut timers = Timers::new();

    // Add a deadline to all slots and the overflow list.
    for n in 0..=SLOTS {
        let deadline = timers.epoch + Duration::from_nanos((n as u64 * NS_PER_SLOT as u64) + 10);
        timers.add(ProcessId(n), deadline);
    }

    let first_deadline = timers.epoch + Duration::from_nanos(10);
    assert_eq!(timers.next(), Some(first_deadline));
    assert_eq!(timers.remove_next(timers.epoch), None);
    assert_eq!(timers.index, 0);

    let mut expected_next_deadline = first_deadline;
    let mut expected_index = 0;
    for n in 0..=SLOTS {
        assert_eq!(timers.next(), Some(expected_next_deadline));
        let now = expected_next_deadline + Duration::from_nanos(1);
        assert_eq!(timers.remove_next(now), Some(ProcessId(n)));
        assert_eq!(timers.index, expected_index);
        assert_eq!(timers.remove_next(now), None);
        assert_eq!(timers.index, expected_index);

        expected_index = (expected_index + 1) % SLOTS as u8;
        expected_next_deadline += DURATION_PER_SLOT;
    }
}

#[test]
fn add_deadline_in_the_past() {
    let mut timers = Timers::new();
    timers.add(PID, timers.epoch - Duration::from_secs(1));
    assert_eq!(timers.next(), Some(timers.epoch));
    assert_eq!(timers.remove_next(timers.epoch), Some(PID));
}

#[test]
fn adding_earlier_deadline_updates_cache() {
    let mut timers = Timers::new();
    let deadline1 = timers.epoch + Duration::from_secs(2);
    let deadline2 = timers.epoch + Duration::from_secs(1);
    timers.add(PID, deadline1);
    timers.add(PID2, deadline2);
    assert_eq!(timers.next(), Some(deadline2));
    assert_eq!(timers.remove_next(deadline1), Some(PID2));
    assert_eq!(timers.remove_next(deadline1), Some(PID));
    assert_eq!(timers.remove_next(deadline1), None);
}

#[test]
fn remove_deadline() {
    let mut timers = Timers::new();
    let deadline = timers.epoch + Duration::from_millis(10);
    timers.add(PID, deadline);
    timers.remove(PID, deadline);
    assert_eq!(timers.next(), None);
    assert_eq!(timers.remove_next(deadline), None);
}

#[test]
fn remove_never_added_deadline() {
    let mut timers = Timers::new();
    let deadline = timers.epoch + Duration::from_millis(10);
    assert_eq!(timers.next(), None);
    assert_eq!(timers.remove_next(deadline), None);
    timers.remove(PID, deadline);
    assert_eq!(timers.next(), None);
    assert_eq!(timers.remove_next(deadline), None);
}

#[test]
fn remove_expired_deadline() {
    let mut timers = Timers::new();
    let deadline = timers.epoch + Duration::from_millis(10);
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
    let mut timers = Timers::new();

    // Add a deadline to all slots and the overflow list.
    for n in 0..=SLOTS {
        let deadline = timers.epoch + Duration::from_nanos((n as u64 * NS_PER_SLOT as u64) + 10);
        timers.add(ProcessId(n), deadline);
    }

    let first_deadline = timers.epoch + Duration::from_nanos(10);
    assert_eq!(timers.next(), Some(first_deadline));
    assert_eq!(timers.remove_next(timers.epoch), None);
    assert_eq!(timers.index, 0);

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
    let mut timers = Timers::new();

    // Add a deadline to all slots and the overflow list.
    for n in 0..=SLOTS {
        let deadline = timers.epoch + Duration::from_nanos((n as u64 * NS_PER_SLOT as u64) + 10);
        timers.add(ProcessId(n), deadline);
        timers.remove(ProcessId(n), deadline);
    }

    assert_eq!(timers.next(), None);
    assert_eq!(timers.remove_next(timers.epoch), None);
    assert_eq!(timers.index, 0);
}

#[test]
fn remove_deadline_after_epoch_advance() {
    let mut timers = Timers::new();

    // Add a deadline to all slots and the overflow list.
    for n in 0..=SLOTS {
        let deadline = timers.epoch + Duration::from_nanos((n as u64 * NS_PER_SLOT as u64) + 10);
        timers.add(ProcessId(n), deadline);
    }

    let first_deadline = timers.epoch + Duration::from_nanos(10);
    let now = timers.epoch + DURATION_PER_SLOT;
    assert_eq!(timers.next(), Some(first_deadline));
    assert_eq!(timers.remove_next(now), Some(ProcessId(0)));
    assert_eq!(timers.remove_next(now), None);
    assert_eq!(timers.index, 1);
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
    let mut timers = Timers::new();
    let deadline = timers.epoch - Duration::from_secs(1);
    timers.add(PID, deadline);
    assert_eq!(timers.next(), Some(timers.epoch));
    timers.remove(PID, deadline);
    assert_eq!(timers.next(), None);
    assert_eq!(timers.remove_next(timers.epoch), None);
}

#[test]
fn change_deadline() {
    let mut timers = Timers::new();
    let deadline = timers.epoch + Duration::from_millis(10);
    timers.add(PID, deadline);
    timers.change(PID, deadline, PID2);
    assert_eq!(timers.next(), Some(deadline));
    assert_eq!(timers.remove_next(deadline), Some(PID2));
    assert_eq!(timers.remove_next(deadline), None);
}

#[test]
fn changing_never_added_deadline_adds_it() {
    let mut timers = Timers::new();
    let deadline = timers.epoch + Duration::from_millis(10);
    timers.change(PID, deadline, PID2);
    assert_eq!(timers.next(), Some(deadline));
    assert_eq!(timers.remove_next(deadline), Some(PID2));
}

#[test]
fn change_expired_deadline() {
    let mut timers = Timers::new();
    let deadline = timers.epoch + Duration::from_millis(10);
    timers.add(PID, deadline);
    assert_eq!(timers.next(), Some(deadline));
    assert_eq!(timers.remove_next(deadline), Some(PID));
    assert_eq!(timers.remove_next(deadline), None);
    timers.change(PID, deadline, PID2);
    assert_eq!(timers.next(), Some(deadline));
    assert_eq!(timers.remove_next(deadline), Some(PID2));
}

#[test]
fn change_deadline_from_all_slots() {
    let mut timers = Timers::new();

    // Add a deadline to all slots and the overflow list.
    for n in 0..=SLOTS {
        let deadline = timers.epoch + Duration::from_nanos((n as u64 * NS_PER_SLOT as u64) + 10);
        timers.add(ProcessId(n), deadline);
    }

    let first_deadline = timers.epoch + Duration::from_nanos(10);
    assert_eq!(timers.next(), Some(first_deadline));
    assert_eq!(timers.remove_next(timers.epoch), None);
    assert_eq!(timers.index, 0);

    let mut next_deadline = first_deadline;
    for n in 0..=SLOTS {
        timers.change(ProcessId(n), next_deadline, ProcessId(100 + n));
        assert_eq!(timers.remove_next(next_deadline), Some(ProcessId(100 + n)));
        assert_eq!(timers.remove_next(next_deadline), None);
        next_deadline += DURATION_PER_SLOT;

        if n == SLOTS {
            assert_eq!(timers.next(), None);
        } else {
            assert_eq!(timers.next(), Some(next_deadline));
        }
    }
}

#[test]
fn change_deadline_from_all_slots_interleaved() {
    let mut timers = Timers::new();

    // Add a deadline to all slots and the overflow list.
    for n in 0..=SLOTS {
        let deadline = timers.epoch + Duration::from_nanos((n as u64 * NS_PER_SLOT as u64) + 10);
        timers.add(ProcessId(n), deadline);
        timers.change(ProcessId(n), deadline, ProcessId(100 + n));
    }

    let now = timers.epoch + Duration::from_nanos((SLOTS as u64 * NS_PER_SLOT as u64) + 10);

    let mut expected_index = 0;
    for n in 0..=SLOTS {
        assert_eq!(timers.remove_next(now), Some(ProcessId(100 + n)));
        assert_eq!(timers.index, expected_index);
        expected_index = (expected_index + 1) % SLOTS as u8;
    }
    assert_eq!(timers.index, 0);
}

#[test]
fn change_deadline_after_epoch_advance() {
    let mut timers = Timers::new();

    // Add a deadline to all slots and the overflow list.
    for n in 0..=SLOTS {
        let deadline = timers.epoch + Duration::from_nanos((n as u64 * NS_PER_SLOT as u64) + 10);
        timers.add(ProcessId(n), deadline);
    }

    let first_deadline = timers.epoch + Duration::from_nanos(10);
    let now = timers.epoch + DURATION_PER_SLOT;
    assert_eq!(timers.next(), Some(first_deadline));
    assert_eq!(timers.remove_next(now), Some(ProcessId(0)));
    assert_eq!(timers.remove_next(now), None);
    assert_eq!(timers.index, 1);
    assert_eq!(timers.next(), Some(first_deadline + DURATION_PER_SLOT));

    let mut next_deadline = first_deadline + DURATION_PER_SLOT;
    for n in 1..=SLOTS {
        timers.change(ProcessId(n), next_deadline, ProcessId(100 + n));
        assert_eq!(timers.remove_next(next_deadline), Some(ProcessId(100 + n)));
        assert_eq!(timers.remove_next(next_deadline), None);
        next_deadline += DURATION_PER_SLOT;

        if n == SLOTS {
            assert_eq!(timers.next(), None);
        } else {
            assert_eq!(timers.next(), Some(next_deadline));
        }
    }
}

#[test]
fn change_deadline_in_the_past() {
    let mut timers = Timers::new();
    let deadline = timers.epoch - Duration::from_secs(1);
    timers.add(PID, deadline);
    assert_eq!(timers.next(), Some(timers.epoch));
    timers.change(PID, deadline, PID2);
    assert_eq!(timers.next(), Some(timers.epoch));
    assert_eq!(timers.remove_next(timers.epoch), Some(PID2));
}

#[test]
fn changing_never_added_deadline_in_the_past_adds_it() {
    let mut timers = Timers::new();
    let deadline = timers.epoch - Duration::from_secs(1);
    timers.change(PID, deadline, PID2);
    assert_eq!(timers.next(), Some(timers.epoch));
    assert_eq!(timers.remove_next(timers.epoch), Some(PID2));
}

#[test]
fn deadlines() {
    let mut timers = Timers::new();

    for n in 0..=SLOTS {
        let deadline = timers.epoch + Duration::from_nanos((n as u64 * NS_PER_SLOT as u64) + 10);
        timers.add(ProcessId(n), deadline);
    }

    let deadlines = timers.deadlines(timers.epoch + OVERFLOW_DURATION + DURATION_PER_SLOT);
    let mut n = 0;
    for pid in deadlines {
        assert_eq!(pid, ProcessId(n));
        n += 1;
    }
    assert_eq!(n, SLOTS + 1);
}

#[test]
fn empty_deadlines() {
    let mut timers = Timers::new();
    let mut deadline = timers.deadlines(timers.epoch);
    assert_eq!(deadline.next(), None);
}

#[test]
fn deadlines_not_yet_expired() {
    let mut timers = Timers::new();
    timers.add(PID, timers.epoch + Duration::from_secs(1));
    let mut deadline = timers.deadlines(timers.epoch);
    assert_eq!(deadline.next(), None);
}
