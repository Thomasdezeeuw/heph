use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Wake, Waker};
use std::time::Duration;

use crate::timers::{TimerToken, Timers, DURATION_PER_SLOT, NS_PER_SLOT, SLOTS};

struct WakerBuilder<const N: usize> {
    awoken: Arc<[AtomicBool; N]>,
    n: usize,
}

impl<const N: usize> WakerBuilder<N> {
    fn new() -> WakerBuilder<N> {
        const FALSE: AtomicBool = AtomicBool::new(false);
        WakerBuilder {
            awoken: Arc::new([FALSE; N]),
            n: 0,
        }
    }

    fn task_waker(&mut self) -> (usize, Waker) {
        let n = self.n;
        self.n += 1;
        assert!(n <= N, "created too many task::Wakers");
        (
            n,
            Waker::from(Arc::new(TaskWaker {
                awoken: self.awoken.clone(),
                n,
            })),
        )
    }

    fn is_awoken(&self, n: usize) -> bool {
        self.awoken[n].load(Ordering::Acquire)
    }
}

/// [`Wake`] implementation.
struct TaskWaker<const N: usize> {
    awoken: Arc<[AtomicBool; N]>,
    n: usize,
}

impl<const N: usize> Wake for TaskWaker<N> {
    fn wake(self: Arc<Self>) {
        self.awoken[self.n].store(true, Ordering::Release)
    }
}

#[test]
fn add_deadline_first_slot() {
    let mut timers = Timers::new();
    let mut wakers = WakerBuilder::<1>::new();

    let deadline = timers.epoch + Duration::from_millis(100);
    let (n, waker) = wakers.task_waker();
    _ = timers.add(deadline, waker);
    assert_eq!(timers.next(), Some(deadline));

    // Not yet expired.
    assert_eq!(timers.expire_timers(timers.epoch), 0);

    // Waker is called when the deadline is expired.
    assert_eq!(timers.expire_timers(deadline), 1);
    assert!(wakers.is_awoken(n));

    // No more timers.
    assert_eq!(timers.expire_timers(deadline), 0);
}

#[test]
fn add_deadline_second_slot() {
    let mut timers = Timers::new();
    let mut wakers = WakerBuilder::<1>::new();

    let deadline = timers.epoch + Duration::from_nanos(NS_PER_SLOT as u64 + 10);
    let (n, waker) = wakers.task_waker();
    _ = timers.add(deadline, waker);
    assert_eq!(timers.next(), Some(deadline));

    assert_eq!(timers.expire_timers(timers.epoch), 0);
    assert_eq!(timers.index, 0);
    assert_eq!(timers.next(), Some(deadline));

    assert_eq!(timers.expire_timers(deadline), 1);
    assert!(wakers.is_awoken(n));
    assert_eq!(timers.index, 1);

    assert_eq!(timers.expire_timers(timers.epoch), 0);
}

#[test]
fn add_deadline_overflow() {
    let mut timers = Timers::new();
    let mut wakers = WakerBuilder::<1>::new();

    let deadline = timers.epoch + Duration::from_nanos(SLOTS as u64 * NS_PER_SLOT as u64 + 10);
    let (n, waker) = wakers.task_waker();
    _ = timers.add(deadline, waker);
    assert_eq!(timers.next(), Some(deadline));

    assert_eq!(timers.expire_timers(timers.epoch), 0);
    assert_eq!(timers.index, 0);
    assert_eq!(timers.next(), Some(deadline));

    assert_eq!(timers.expire_timers(deadline), 1);
    assert!(wakers.is_awoken(n));
    // Should have advanced the epoch to come back around to 0.
    assert_eq!(timers.index, 0);

    assert_eq!(timers.expire_timers(timers.epoch), 0);
}

#[test]
fn add_deadline_to_all_slots() {
    let mut timers = Timers::new();
    let mut wakers = WakerBuilder::<{ SLOTS + 1 }>::new();

    // Add a deadline to all slots and the overflow list.
    for n in 0..=SLOTS {
        let deadline = timers.epoch + Duration::from_nanos((n as u64 * NS_PER_SLOT as u64) + 10);
        let (n2, waker) = wakers.task_waker();
        assert_eq!(n, n2);
        _ = timers.add(deadline, waker);
    }

    let first_deadline = timers.epoch + Duration::from_nanos(10);
    assert_eq!(timers.next(), Some(first_deadline));
    assert_eq!(timers.expire_timers(timers.epoch), 0);
    assert_eq!(timers.index, 0);

    let mut expected_next_deadline = first_deadline;
    let mut expected_index = 0;
    for n in 0..=SLOTS {
        assert_eq!(timers.next(), Some(expected_next_deadline));
        let now = expected_next_deadline + Duration::from_nanos(1);
        assert_eq!(timers.expire_timers(now), 1);
        assert!(wakers.is_awoken(n));
        assert_eq!(timers.index, expected_index);

        assert_eq!(timers.expire_timers(timers.epoch), 0);
        assert_eq!(timers.index, expected_index);

        expected_index = (expected_index + 1) % SLOTS as u8;
        expected_next_deadline += DURATION_PER_SLOT;
    }
}

#[test]
fn add_deadline_in_the_past() {
    let mut timers = Timers::new();
    let mut wakers = WakerBuilder::<1>::new();

    let (n, waker) = wakers.task_waker();
    _ = timers.add(timers.epoch - Duration::from_secs(1), waker);
    assert_eq!(timers.next(), Some(timers.epoch));

    assert_eq!(timers.expire_timers(timers.epoch), 1);
    assert!(wakers.is_awoken(n));
}

#[test]
fn adding_earlier_deadline_updates_cache() {
    let mut timers = Timers::new();
    let mut wakers = WakerBuilder::<2>::new();

    let deadline1 = timers.epoch + Duration::from_secs(2);
    let (n1, waker) = wakers.task_waker();
    _ = timers.add(deadline1, waker);
    let deadline2 = timers.epoch + Duration::from_secs(1);
    let (n2, waker) = wakers.task_waker();
    _ = timers.add(deadline2, waker);
    assert_eq!(timers.next(), Some(deadline2));

    assert_eq!(timers.expire_timers(deadline1), 2);
    assert!(wakers.is_awoken(n1));
    assert!(wakers.is_awoken(n2));
    assert_eq!(timers.expire_timers(deadline1), 0);
}

#[test]
fn remove_deadline() {
    let mut timers = Timers::new();
    let mut wakers = WakerBuilder::<1>::new();

    let deadline = timers.epoch + Duration::from_millis(10);
    let (n, waker) = wakers.task_waker();
    let token = timers.add(deadline, waker);
    timers.remove(deadline, token);
    assert_eq!(timers.next(), None);
    assert_eq!(timers.expire_timers(timers.epoch), 0);
    assert!(!wakers.is_awoken(n));
}

#[test]
fn remove_never_added_deadline() {
    let mut timers = Timers::new();

    let deadline = timers.epoch + Duration::from_millis(10);
    assert_eq!(timers.next(), None);
    assert_eq!(timers.expire_timers(timers.epoch), 0);
    timers.remove(deadline, TimerToken(0));
    assert_eq!(timers.next(), None);
    assert_eq!(timers.expire_timers(timers.epoch), 0);
}

#[test]
fn remove_expired_deadline() {
    let mut timers = Timers::new();
    let mut wakers = WakerBuilder::<1>::new();

    let deadline = timers.epoch + Duration::from_millis(10);
    let (n, waker) = wakers.task_waker();
    let token = timers.add(deadline, waker);

    assert_eq!(timers.next(), Some(deadline));
    assert_eq!(timers.expire_timers(deadline), 1);
    assert!(wakers.is_awoken(n));
    assert_eq!(timers.expire_timers(deadline), 0);

    timers.remove(deadline, token);
    assert_eq!(timers.next(), None);
    assert_eq!(timers.expire_timers(deadline), 0);
}

#[test]
fn remove_deadline_from_all_slots() {
    let mut timers = Timers::new();
    let mut wakers = WakerBuilder::<{ SLOTS + 1 }>::new();

    // Add a deadline to all slots and the overflow list.
    let tokens: Vec<TimerToken> = (0..=SLOTS)
        .into_iter()
        .map(|n| {
            let deadline =
                timers.epoch + Duration::from_nanos((n as u64 * NS_PER_SLOT as u64) + 10);
            let (n2, waker) = wakers.task_waker();
            assert_eq!(n2, n);
            timers.add(deadline, waker)
        })
        .collect();

    let first_deadline = timers.epoch + Duration::from_nanos(10);
    assert_eq!(timers.next(), Some(first_deadline));
    assert_eq!(timers.expire_timers(timers.epoch), 0);
    assert_eq!(timers.index, 0);

    let mut next_deadline = first_deadline;
    for (n, token) in tokens.into_iter().enumerate() {
        timers.remove(next_deadline, token);
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
    let mut wakers = WakerBuilder::<{ SLOTS + 1 }>::new();

    // Add a deadline to all slots and the overflow list.
    for n in 0..=SLOTS {
        let deadline = timers.epoch + Duration::from_nanos((n as u64 * NS_PER_SLOT as u64) + 10);
        let (n2, waker) = wakers.task_waker();
        assert_eq!(n2, n);
        let token = timers.add(deadline, waker);
        timers.remove(deadline, token);
    }

    assert_eq!(timers.next(), None);
    assert_eq!(timers.expire_timers(timers.epoch), 0);
    assert_eq!(timers.index, 0);
}

#[test]
fn remove_deadline_after_epoch_advance() {
    let mut timers = Timers::new();
    let mut wakers = WakerBuilder::<{ SLOTS + 1 }>::new();

    // Add a deadline to all slots and the overflow list.
    let tokens: Vec<TimerToken> = (0..=SLOTS)
        .into_iter()
        .map(|n| {
            let deadline =
                timers.epoch + Duration::from_nanos((n as u64 * NS_PER_SLOT as u64) + 10);
            let (n2, waker) = wakers.task_waker();
            assert_eq!(n2, n);
            timers.add(deadline, waker)
        })
        .collect();

    let first_deadline = timers.epoch + Duration::from_nanos(10);
    let now = timers.epoch + DURATION_PER_SLOT;
    assert_eq!(timers.next(), Some(first_deadline));
    assert_eq!(timers.expire_timers(now), 1);
    assert!(wakers.is_awoken(0));
    assert_eq!(timers.expire_timers(timers.epoch), 0);
    assert_eq!(timers.index, 1);
    assert_eq!(timers.next(), Some(first_deadline + DURATION_PER_SLOT));

    let mut next_deadline = first_deadline + DURATION_PER_SLOT;
    for (n, token) in tokens.into_iter().skip(1).enumerate() {
        timers.remove(next_deadline, token);
        next_deadline += DURATION_PER_SLOT;

        if n == SLOTS - 1 {
            assert_eq!(timers.next(), None);
        } else {
            assert_eq!(timers.next(), Some(next_deadline));
        }
    }
}

#[test]
fn remove_deadline_in_the_past() {
    let mut timers = Timers::new();
    let mut wakers = WakerBuilder::<1>::new();

    let deadline = timers.epoch - Duration::from_secs(1);
    let (n, waker) = wakers.task_waker();
    let token = timers.add(deadline, waker);
    assert_eq!(timers.next(), Some(timers.epoch));
    timers.remove(deadline, token);
    assert_eq!(timers.next(), None);
    assert_eq!(timers.expire_timers(timers.epoch), 0);
    assert!(!wakers.is_awoken(n));
}

mod shared {
    use std::time::Duration;

    use crate::timers::shared::Timers;
    use crate::timers::tests::WakerBuilder;
    use crate::timers::{TimerToken, DURATION_PER_SLOT, NS_PER_SLOT, SLOTS};

    #[test]
    fn add_deadline_first_slot() {
        let timers = Timers::new();
        let mut wakers = WakerBuilder::<1>::new();
        let epoch = timers.epoch().0;
        let deadline = epoch + Duration::from_millis(100);

        let (n, waker) = wakers.task_waker();
        _ = timers.add(deadline, waker);
        assert_eq!(timers.next(), Some(deadline));

        // Not yet expired.
        assert_eq!(timers.expire_timers(epoch), 0);
        assert!(!wakers.is_awoken(n));

        // Waker is called when the deadline is expired.
        assert_eq!(timers.expire_timers(deadline), 1);
        assert!(wakers.is_awoken(n));

        // No more timers.
        assert_eq!(timers.expire_timers(deadline + Duration::from_secs(100)), 0);
    }

    #[test]
    fn add_deadline_second_slot() {
        let timers = Timers::new();
        let mut wakers = WakerBuilder::<1>::new();
        let epoch = timers.epoch().0;

        let deadline = epoch + Duration::from_nanos(NS_PER_SLOT as u64 + 100);
        let (n, waker) = wakers.task_waker();
        _ = timers.add(deadline, waker);
        assert_eq!(timers.next(), Some(deadline));

        assert_eq!(timers.expire_timers(epoch), 0);
        assert_eq!(timers.epoch().1, 0);
        assert_eq!(timers.next(), Some(deadline));

        assert_eq!(timers.expire_timers(deadline), 1);
        assert!(wakers.is_awoken(n));

        assert_eq!(timers.epoch().1, 1);
        assert_eq!(timers.expire_timers(epoch), 0);
    }

    #[test]
    fn add_deadline_overflow() {
        let timers = Timers::new();
        let mut wakers = WakerBuilder::<1>::new();
        let epoch = timers.epoch().0;

        let deadline = epoch + Duration::from_nanos(SLOTS as u64 * NS_PER_SLOT as u64 + 10);
        let (n, waker) = wakers.task_waker();
        _ = timers.add(deadline, waker);
        assert_eq!(timers.next(), Some(deadline));

        assert_eq!(timers.expire_timers(epoch), 0);
        assert_eq!(timers.epoch().1, 0);
        assert_eq!(timers.next(), Some(deadline));

        assert_eq!(timers.expire_timers(deadline), 1);
        assert!(wakers.is_awoken(n));

        // Should have advanced the epoch to come back around to 0.
        assert_eq!(timers.epoch().1, 0);
        assert_eq!(timers.expire_timers(epoch), 0);
    }

    #[test]
    fn add_deadline_to_all_slots() {
        let timers = Timers::new();
        let mut wakers = WakerBuilder::<{ SLOTS + 1 }>::new();
        let epoch = timers.epoch().0;

        // Add a deadline to all slots and the overflow list.
        for n in 0..=SLOTS {
            let deadline = epoch + Duration::from_nanos((n as u64 * NS_PER_SLOT as u64) + 10);
            let (n2, waker) = wakers.task_waker();
            assert_eq!(n, n2);
            _ = timers.add(deadline, waker);
        }

        let first_deadline = epoch + Duration::from_nanos(10);
        assert_eq!(timers.next(), Some(first_deadline));
        assert_eq!(timers.expire_timers(epoch), 0);
        assert_eq!(timers.epoch().1, 0);

        let mut expected_next_deadline = first_deadline;
        let mut expected_index = 0;
        for n in 0..=SLOTS {
            assert_eq!(timers.next(), Some(expected_next_deadline));
            let now = expected_next_deadline + Duration::from_nanos(1);
            assert_eq!(timers.expire_timers(now), 1);
            assert!(wakers.is_awoken(n));
            assert_eq!(timers.epoch().1, expected_index);

            assert_eq!(timers.expire_timers(now), 0);
            assert_eq!(timers.epoch().1, expected_index);

            expected_index = (expected_index + 1) % SLOTS as u8;
            expected_next_deadline += DURATION_PER_SLOT;
        }
    }

    #[test]
    fn add_deadline_in_the_past() {
        let timers = Timers::new();
        let mut wakers = WakerBuilder::<1>::new();
        let epoch = timers.epoch().0;

        let (n, waker) = wakers.task_waker();
        _ = timers.add(epoch - Duration::from_secs(1), waker);
        assert_eq!(timers.next(), Some(epoch));

        assert_eq!(timers.expire_timers(epoch), 1);
        assert!(wakers.is_awoken(n));
    }

    #[test]
    fn adding_earlier_deadline() {
        let timers = Timers::new();
        let mut wakers = WakerBuilder::<2>::new();
        let epoch = timers.epoch().0;

        let (n1, waker) = wakers.task_waker();
        let deadline1 = epoch + Duration::from_secs(2);
        _ = timers.add(deadline1, waker);
        assert_eq!(timers.next(), Some(deadline1));

        let (n2, waker) = wakers.task_waker();
        let deadline2 = epoch + Duration::from_secs(1);
        _ = timers.add(deadline2, waker);
        assert_eq!(timers.next(), Some(deadline2));

        assert_eq!(timers.expire_timers(deadline1), 2);
        assert!(wakers.is_awoken(n1));
        assert!(wakers.is_awoken(n2));
        assert_eq!(timers.expire_timers(deadline1), 0);
    }

    #[test]
    fn remove_deadline() {
        let timers = Timers::new();
        let mut wakers = WakerBuilder::<1>::new();
        let epoch = timers.epoch().0;

        let deadline = epoch + Duration::from_millis(10);
        let (_, waker) = wakers.task_waker();
        let token = timers.add(deadline, waker);
        assert_eq!(timers.next(), Some(deadline));

        timers.remove(deadline, token);
        assert_eq!(timers.next(), None);
        assert_eq!(timers.expire_timers(epoch), 0);
    }

    #[test]
    fn remove_never_added_deadline() {
        let timers = Timers::new();
        let epoch = timers.epoch().0;

        assert_eq!(timers.next(), None);
        assert_eq!(timers.expire_timers(epoch), 0);
        let deadline = epoch + Duration::from_millis(10);
        timers.remove(deadline, TimerToken(0));
        assert_eq!(timers.next(), None);
        assert_eq!(timers.expire_timers(epoch), 0);
    }

    #[test]
    fn remove_expired_deadline() {
        let timers = Timers::new();
        let mut wakers = WakerBuilder::<1>::new();
        let epoch = timers.epoch().0;

        let deadline = epoch + Duration::from_millis(10);
        let (n, waker) = wakers.task_waker();
        let token = timers.add(deadline, waker);
        assert_eq!(timers.next(), Some(deadline));

        assert_eq!(timers.expire_timers(deadline), 1);
        assert!(wakers.is_awoken(n));

        timers.remove(deadline, token);
        assert_eq!(timers.next(), None);
        assert_eq!(timers.expire_timers(epoch), 0);
    }

    #[test]
    fn remove_deadline_from_all_slots() {
        let timers = Timers::new();
        let mut wakers = WakerBuilder::<{ SLOTS + 1 }>::new();
        let epoch = timers.epoch().0;

        // Add a deadline to all slots and the overflow list.
        let tokens: Vec<TimerToken> = (0..=SLOTS)
            .into_iter()
            .map(|n| {
                let deadline = epoch + Duration::from_nanos((n as u64 * NS_PER_SLOT as u64) + 10);
                let (n2, waker) = wakers.task_waker();
                assert_eq!(n, n2);
                timers.add(deadline, waker)
            })
            .collect();

        let first_deadline = epoch + Duration::from_nanos(10);
        assert_eq!(timers.next(), Some(first_deadline));
        assert_eq!(timers.expire_timers(epoch), 0);
        assert_eq!(timers.epoch().1, 0);

        let mut next_deadline = first_deadline;
        for (n, token) in tokens.into_iter().enumerate() {
            timers.remove(next_deadline, token);
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
        let mut wakers = WakerBuilder::<{ SLOTS + 1 }>::new();
        let epoch = timers.epoch().0;

        // Add a deadline to all slots and the overflow list.
        for n in 0..=SLOTS {
            let deadline = epoch + Duration::from_nanos((n as u64 * NS_PER_SLOT as u64) + 10);
            let (n2, waker) = wakers.task_waker();
            assert_eq!(n, n2);
            let token = timers.add(deadline, waker);
            timers.remove(deadline, token);
        }

        assert_eq!(timers.next(), None);
        assert_eq!(timers.expire_timers(epoch), 0);
        assert_eq!(timers.epoch().1, 0);
    }

    #[test]
    fn remove_deadline_after_epoch_advance() {
        let timers = Timers::new();
        let mut wakers = WakerBuilder::<{ SLOTS + 1 }>::new();
        let epoch = timers.epoch().0;

        // Add a deadline to all slots and the overflow list.
        let tokens: Vec<TimerToken> = (0..=SLOTS)
            .into_iter()
            .map(|n| {
                let deadline = epoch + Duration::from_nanos((n as u64 * NS_PER_SLOT as u64) + 10);
                let (n2, waker) = wakers.task_waker();
                assert_eq!(n, n2);
                timers.add(deadline, waker)
            })
            .skip(1)
            .collect();

        let first_deadline = epoch + Duration::from_nanos(10);
        let now = epoch + DURATION_PER_SLOT;
        assert_eq!(timers.next(), Some(first_deadline));
        assert_eq!(timers.expire_timers(now), 1);
        assert!(wakers.is_awoken(0));
        assert_eq!(timers.epoch().1, 1);
        assert_eq!(timers.next(), Some(first_deadline + DURATION_PER_SLOT));

        let mut next_deadline = first_deadline + DURATION_PER_SLOT;
        for (n, token) in tokens.into_iter().enumerate() {
            timers.remove(next_deadline, token);
            next_deadline += DURATION_PER_SLOT;

            if n == SLOTS - 1 {
                assert_eq!(timers.next(), None);
            } else {
                assert_eq!(timers.next(), Some(next_deadline));
            }
        }
    }

    #[test]
    fn remove_deadline_in_the_past() {
        let timers = Timers::new();
        let mut wakers = WakerBuilder::<1>::new();
        let epoch = timers.epoch().0;

        let deadline = epoch - Duration::from_secs(1);
        let (_, waker) = wakers.task_waker();
        let token = timers.add(deadline, waker);
        assert_eq!(timers.next(), Some(epoch));

        timers.remove(deadline, token);
        assert_eq!(timers.next(), None);
        assert_eq!(timers.expire_timers(epoch), 0);
    }
}
